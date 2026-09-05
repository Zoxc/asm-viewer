//! The app's root state: every context provided once at the root of `app()` and read with
//! `use_consume` wherever it is wanted.
//!
//! Two of the names are **derivations and not states**: `Active` is a `Memo` over the strip
//! and the document table, and `Symbols` a `Memo` over `Objects`.

use super::*;

/// The loaded objects, shared through context.
#[derive(Clone, Copy)]
pub(crate) struct Objects(pub(crate) State<Vec<Arc<Object>>>);

/// The files being read into [`Objects`] right now, so the sidebar can say so. A state of
/// its own because it is about what that list has *not* got: a file appears here when it
/// is asked for and leaves when nothing more is coming out of it, whether or not it
/// produced anything at all. See [`Loads`] and [`open_binaries`].
#[derive(Clone, Copy)]
pub(crate) struct Loading(pub(crate) State<Loads>);

/// The active tab and the document it shows, shared through context.
///
/// **A derivation and not a state**: the strip's active tab, read through [`Docs`] -- see
/// [`active_tab`]. `None` means both "nothing is open" and "the tab on screen is a page",
/// and deliberately does not distinguish them. The id travels with the document because
/// the two are read together: the driven line and the viewing positions are kept per tab
/// *and* entry, and a document paired with an id read a beat apart would be another tab's
/// for that beat, which the worker would answer with a re-ask.
///
/// A [`Memo`] because the strip is written by more than the opening of a document -- a
/// page raised, a tab moved along the bar, a page closed -- and none of those changes what
/// any pane is drawing. **It is therefore a beat behind**, which is right for
/// anything that renders and wrong for anything that must be true inside one event
/// handler -- so the functions holding the invariants call [`active_tab`] on the states
/// directly instead of reading this.
#[derive(Clone, Copy)]
pub(crate) struct Active(pub(crate) Memo<Option<Entry>>);

/// What is open: the strip of tabs the reader arranged, and the table saying what each
/// document tab stands for.
///
/// The strip's tabs *are* the list of open tabs, in the reader's own order; [`Docs`] holds
/// no order, only the trail behind each document tab's id. Membership is the one thing the
/// two share, and `open_document`/`close_tab`/`close_binary` keep it true: a tab and its
/// trail are made together and closed together.
#[derive(Clone, Copy)]
pub(crate) struct Open {
    pub(crate) strip: State<Strip>,
    pub(crate) docs: State<Docs>,
}

/// Every open tab's document, in the order the reader's tabs are in. Pages are skipped:
/// they are tabs in the same bar but they are not documents. What the tests ask of the
/// strip; the app itself asks for the ids ([`open_ids`]), a tab being a trail and not
/// what it shows.
#[cfg(test)]
pub(crate) fn open_documents(strip: &Strip, docs: &Docs) -> Vec<Document> {
    strip
        .documents()
        .filter_map(|id| docs.get(id).cloned())
        .collect()
}

/// Every open document tab's id, in the order the reader's tabs are in.
pub(crate) fn open_ids(strip: &Strip) -> Vec<DocId> {
    strip.documents().collect()
}

/// The active tab and what it shows: the tab on screen, when that tab is a document.
pub(crate) fn active_tab(strip: &Strip, docs: &Docs) -> Option<Entry> {
    match strip.active()? {
        Tab::Document(id) => docs.current(id).cloned().map(|stop| (id, stop)),
        Tab::Page(_) => None,
    }
}

/// The active document alone.
pub(crate) fn active_document(strip: &Strip, docs: &Docs) -> Option<Document> {
    active_tab(strip, docs).map(|(_, stop)| stop.document)
}

impl Open {
    /// The active document as of *now*, for the event handlers that cannot wait a beat
    /// for [`Active`] to catch up. `peek`, so asking subscribes nothing.
    pub(crate) fn active(&self) -> Option<Document> {
        self.active_stop().map(|(_, stop)| stop.document)
    }

    /// The active tab and the place on its trail it is at, as of now. The document is
    /// what most callers want ([`Open::active`]); this is for the few that key by place.
    pub(crate) fn active_stop(&self) -> Option<Entry> {
        let (strip, docs) = (self.strip.peek(), self.docs.peek());
        active_tab(&strip, &docs)
    }

    /// The active tab as of now, with the document it shows. `peek`, for the same reason.
    pub(crate) fn active_tab(&self) -> Option<(DocId, Document)> {
        self.active_stop().map(|(id, stop)| (id, stop.document))
    }

    /// The active tab's id as of now, a document or not.
    pub(crate) fn active_id(&self) -> Option<DocId> {
        self.active_tab().map(|(id, _)| id)
    }

    /// Every open tab's document as of now, in tab order. `peek`, for the same reason.
    #[cfg(test)]
    pub(crate) fn documents(&self) -> Vec<Document> {
        let (strip, docs) = (self.strip.peek(), self.docs.peek());
        open_documents(&strip, &docs)
    }

    /// Every open document tab's id as of now, in tab order.
    pub(crate) fn ids(&self) -> Vec<DocId> {
        open_ids(&self.strip.peek())
    }
}

/// Every box inside the tab on screen the keyboard can be in -- the two code panes, the
/// listing of an object's code, the scratchpad's editor -- and whether a press on a chip
/// has asked for it to go there.
///
/// Each box registers itself while it is mounted ([`use_tab_keyboard`]), and only the tab
/// on screen is mounted, so what this answers is "the keyboard is in the tab and not in the
/// sidebar" -- which is what the mark over the tab on screen says.
#[derive(Clone, Copy)]
pub(crate) struct Keyboard(pub(crate) State<Keys>);

/// The strip of open tabs.
#[derive(Clone, Copy)]
pub(crate) struct OpenTabs(pub(crate) State<Strip>);

/// The sidebar's dock, so a panel can be brought to the front from anywhere -- the
/// Search box's chord, a Locations question asked in a code pane.
#[derive(Clone, Copy)]
pub(crate) struct SidebarDock(pub(crate) State<DockArea>);

/// How wide the **leading** side of a document is, as a percentage -- the side the tab is
/// driven from, which `DocumentBody` draws on the left in both kinds of tab. Kept by place
/// and not by pane, so switching from an assembly-driven tab to a source-driven one leaves
/// the handle where the reader put it instead of throwing the two widths across the split.
///
/// One number for the app, held out here because the container will not remember it: only
/// the active tab's content is mounted, and a `ResizablePanel` registers at its
/// `initial_size` in a `use_hook` and removes its entry in a `use_drop`, so a remount comes
/// back at the initial sizes under new panel ids.
#[derive(Clone, Copy)]
pub(crate) struct SplitRatio(pub(crate) State<f32>);

/// The `ResizableContext` the document's two panels register into, so a drag on the handle
/// can be read back out. See [`SplitRatio`].
#[derive(Clone, Copy)]
pub(crate) struct Splits(pub(crate) State<ResizableContext>);

/// How wide the sidebar is, in pixels. [`SplitRatio`]'s shape and for its reason: a
/// `ResizablePanel` registers at its `initial_size` and forgets on unmount, so a container
/// that is rebuilt -- which the window's body is, whenever a project arrives or goes --
/// comes back at the initial size unless the number is held out here.
///
/// Pixels and not a percentage: this panel is `PanelSize::px`, so what the context holds
/// after a drag is a literal width.
#[derive(Clone, Copy)]
pub(crate) struct SidebarWidth(pub(crate) State<f32>);

/// The `ResizableContext` the sidebar and the content register into. See [`SidebarWidth`].
#[derive(Clone, Copy)]
pub(crate) struct SidebarSplits(pub(crate) State<ResizableContext>);

/// Which row each place on each open tab's trail had its **assembly** side left on. At
/// the root rather than in the pane, which reuses one scroll controller for every symbol
/// and so would leave a newly opened function at the offset the old one was at. Keyed by
/// [`Entry`] -- the tab and the place -- so going back along a trail comes back to the row
/// that was left, and an entry means "this place on this tab" for exactly as long as the
/// tab is open and the place is on its trail. A place and not a document: two addresses
/// in one object's code are two of them, which is what makes a step inside that listing
/// come back to the row it was left at as any other step does.
#[derive(Clone, Copy)]
pub(crate) struct AsmAt(pub(crate) State<Positions<Entry>>);

/// The documents the strip's tabs are handles into, and nothing about their order. See
/// [`Docs`]: it exists because a tab must be `Copy` -- a list's key, a menu row's capture
/// -- and a [`Document`] is not.
#[derive(Clone, Copy)]
pub(crate) struct OpenDocs(pub(crate) State<Docs>);

/// Which row each place's **source** side was left on. [`AsmAt`]'s other half, keyed by
/// the same entry rather than by the file the pane happens to be showing.
#[derive(Clone, Copy)]
pub(crate) struct SrcAt(pub(crate) State<Positions<Entry>>);

/// Which tabs have the section under their Assembly pane's symbol bar open.
///
/// **Per tab and never in the pane**, which is mounted afresh for every document: a
/// `use_state` there would collapse the section at every switch of tab, and a reader who
/// opened it once would find it shut every time they came back.
///
/// **Keyed by [`DocId`] alone and not by [`Entry`]**, unlike [`AsmAt`] and [`Drives`]
/// beside it, and that is what makes it cost nothing: a `DocId` is `Copy + Hash` and
/// holds no `Arc<Object>`, where a document does and would have to be forgotten in all
/// three of `close_tab`, `close_others` and `close_binary` or a closed binary's bytes
/// would be held for as long as the app ran. Ids are never handed out twice
/// ([`Docs::open`]), so an entry a closed tab left behind can never be mistaken for
/// another tab's -- it is dead weight of four bytes, and a reopened tab correctly opens
/// with its section shut. The Objects tree's fold set makes the same argument. It follows
/// that the section stays open or shut across the whole of a tab's trail, which is a
/// fact about the tab and not about any one place on it.
///
/// Never saved: it is a view of a tab, like a filter.
#[derive(Clone, Copy)]
pub(crate) struct Expanded(pub(crate) State<HashSet<DocId>>);

/// What the reader has said about each tab's following pane -- the one it is not driven
/// from: `true` where they brought it back, `false` where they put it away. A tab with
/// nothing here has said nothing, and opens as its document says ([`following`]).
///
/// A `bool` per tab and not the set [`Expanded`] is, because there is no one default to
/// be absent from: a source-driven tab on a `Cargo.toml` opens with its assembly side
/// away and every other tab opens with both panes.
///
/// Keyed by [`DocId`] alone and never saved, for [`Expanded`]'s reasons: an id is
/// `Copy + Hash`, holds no `Arc<Object>` and is never handed out twice, so a closed tab
/// leaves a byte behind that no other tab can be given, and this is a view of a tab.
#[derive(Clone, Copy)]
pub(crate) struct Follows(pub(crate) State<HashMap<DocId, bool>>);

/// Which source line each source-driven tab's assembly side is driven from, shared
/// through context. Beside [`AsmAt`]/[`SrcAt`] because it is the same kind of thing: a
/// fact about a tab, made by a click in it and forgotten with it.
#[derive(Clone, Copy)]
pub(crate) struct Drives(pub(crate) State<Driven>);

/// Everywhere the reader has been, across every tab: what the History panel lists.
#[derive(Clone, Copy)]
pub(crate) struct Visited(pub(crate) State<Visits>);

/// Where the reader chose to be able to come back to: the project's bookmarks, in their
/// saved shape and nothing more. Whether one is live is asked of [`Objects`] where it is
/// drawn, so a closed binary takes no bookmark with it and holds no `Arc` through one.
#[derive(Clone, Copy)]
pub(crate) struct Bookmarked(pub(crate) State<Bookmarks>);

/// The project the app is in, as the project view holds it.
///
/// Two of its fields are `String`s where [`Details`] has `Option`s, because this is what is
/// in two text boxes and a text box has no third state: an empty box *is* how a reader says
/// "I have not said". [`OpenProject::details`] is the one place the two spellings meet.
#[derive(Clone, Default, PartialEq)]
pub(crate) struct OpenProject {
    /// The file the project is kept in, which is its identity. `None` until a project
    /// exists on disk at all.
    pub(crate) file: Option<PathBuf>,
    pub(crate) directory: String,
    /// The language server to read this project with, empty for the usual one. A box like
    /// the one above: a project on a toolchain of its own is the only one that fills it.
    pub(crate) language_server: String,
    /// Whether the reader has agreed to a language server being run over the directory
    /// above. A plain value like the profile below: the prompt has no third answer, and
    /// a project that was never asked is one that has not agreed.
    pub(crate) trusted: bool,
    /// What to build the directory with. A plain value and not an `Option`, since the two
    /// buttons that set it have no third state either; the file is what leaves it out.
    pub(crate) profile: Profile,
}

impl OpenProject {
    /// The project as it was found on disk.
    pub(crate) fn opened(file: PathBuf, project: &Project, trusted: bool) -> OpenProject {
        OpenProject {
            file: Some(file),
            directory: project
                .directory
                .as_ref()
                .map(|directory| directory.to_string_lossy().into_owned())
                .unwrap_or_default(),
            language_server: project.language_server.clone().unwrap_or_default(),
            trusted,
            profile: project.cargo.clone().unwrap_or_default().profile,
        }
    }

    /// The program to read this project with: what the reader named, or the usual one.
    pub(crate) fn server(&self) -> String {
        given(&self.language_server)
            .unwrap_or(lsp::SERVER)
            .to_owned()
    }

    /// What of this reaches the project file. Trimmed, so a box holding nothing but spaces
    /// is a box holding nothing. `trusted` is not here: the agreement is the session's, so
    /// it reaches the disk through [`Session::from_state`] instead.
    pub(crate) fn details(&self) -> Details {
        Details {
            directory: given(&self.directory).map(PathBuf::from),
            language_server: given(&self.language_server).map(str::to_owned),
            // Absent while it says nothing the defaults do not: a reader who has never
            // touched the profile leaves no `[cargo]` behind, and choosing the default
            // back takes the section out again.
            cargo: (self.profile != Profile::default()).then(|| Cargo {
                profile: self.profile,
            }),
        }
    }
}

/// What a text box says, or `None` when it says nothing.
pub(crate) fn given(text: &str) -> Option<&str> {
    let text = text.trim();
    (!text.is_empty()).then_some(text)
}

/// The open project, shared through context.
#[derive(Clone, Copy)]
pub(crate) struct Proj(pub(crate) State<OpenProject>);

/// The user's settings as the settings page has them. [`OpenProject`]'s shape, and for its
/// reason: a family is a `String` here and an `Option<String>` in `Settings`, and
/// [`EditedSettings::settings`] is the one place the two spellings meet. A size is edited
/// by a stepper rather than a text box, so it needs no such treatment.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct EditedSettings {
    pub(crate) theme: ThemeChoice,
    pub(crate) interface: EditedFont,
    pub(crate) fixed: EditedFont,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct EditedFont {
    pub(crate) family: String,
    /// In points, like the file and like [`Font::points`], so the number on screen, the
    /// number the desktop answered and the number written down are one number.
    pub(crate) size: Option<f32>,
}

impl EditedSettings {
    /// The settings as they were read off disk.
    pub(crate) fn of(settings: &Settings) -> EditedSettings {
        EditedSettings {
            theme: settings.theme,
            interface: EditedFont::of(&settings.interface),
            fixed: EditedFont::of(&settings.fixed),
        }
    }

    /// What of this reaches `settings.toml` -- and, through [`fonts::resolve`], what is on
    /// screen.
    pub(crate) fn settings(&self) -> Settings {
        Settings {
            theme: self.theme,
            interface: self.interface.setting(),
            fixed: self.fixed.setting(),
        }
    }
}

impl EditedFont {
    pub(crate) fn of(setting: &FontSetting) -> EditedFont {
        EditedFont {
            family: setting.family().unwrap_or_default().to_owned(),
            size: setting.size(),
        }
    }

    pub(crate) fn setting(&self) -> FontSetting {
        FontSetting {
            family: given(&self.family).map(str::to_owned),
            size: self.size,
        }
    }
}

/// The settings, shared through context. A root context and not state inside the settings
/// page, which is a tab that may not be open at all. The page edits this;
/// `use_settings_with` is what notices.
#[derive(Clone, Copy)]
pub(crate) struct Prefs(pub(crate) State<EditedSettings>);

/// Where each file that would not parse was moved to, until the reader has been told: what
/// [`RescuedPopup`] draws, and empty for every run in which nothing was moved.
///
/// A state at the root and not one inside the popup, because what fills it is a *load*
/// (`rescue::moved`), and the two loads a run makes are the startup's and a project
/// switch's -- neither of them anywhere near a component that could own this.
#[derive(Clone, Copy)]
pub(crate) struct Rescued(pub(crate) State<Vec<PathBuf>>);

/// Every state a project owns, in one `Copy` bundle of handles: a project switch closes
/// all of them and reopens all of them.
#[derive(Clone, Copy)]
pub(crate) struct ProjectStates {
    pub(crate) proj: State<OpenProject>,
    pub(crate) objects: State<Vec<Arc<Object>>>,
    /// The files on their way into `objects`. Leaving a project abandons them too,
    /// including the ones that have produced nothing yet and so are not in `objects` to be
    /// closed one by one.
    pub(crate) loading: State<Loads>,
    /// The strip and the id table: what is open, and in what order.
    pub(crate) open: Open,
    pub(crate) asm_at: State<Positions<Entry>>,
    pub(crate) src_at: State<Positions<Entry>>,
    /// Where each code tab was left, as an address.
    pub(crate) code_at: State<Positions<Entry, Spot>>,
    /// Which line each source-driven tab's assembly side is driven from.
    pub(crate) driven: State<Driven>,
    /// What each place had picked out in each pane when it was last shown. Never saved.
    pub(crate) marks_at: State<Positions<Entry, Kept>>,
    pub(crate) visits: State<Visits>,
    pub(crate) bookmarks: State<Bookmarks>,
    /// What the project's directory was last searched for, and what was found in it.
    pub(crate) searched: State<Searched>,
    /// What the project's own workspace built, and what a build replaces.
    pub(crate) build: State<Builds>,
    /// How the window itself is arranged. Not a project's state, and here all the same:
    /// it is written into the session, and a restore has to put it back.
    pub(crate) arranged: Arrangement,
}

/// The three states a session's `[ui]` is kept in: what a restore writes and what the
/// save observer reads back out.
///
/// **Held as states and not reached for.** They live in contexts of their own, and the one
/// way to a context is `use_consume`, which is a hook -- so a restore that asked for them
/// itself would be calling hooks from wherever it was called from. A restore runs inside
/// `use_hook` at startup and inside a press handler on a switch, and neither may
/// (`src/ui/project_view.rs`, `restore_ui`). Consumed by whoever is rendering and handed
/// down instead.
#[derive(Clone, Copy)]
pub(crate) struct Arrangement {
    pub(crate) dock: State<DockArea>,
    pub(crate) sidebar: State<f32>,
    pub(crate) split: State<f32>,
}

/// What is open, as a component sees it: the strip and the id table together.
pub(crate) fn use_open() -> Open {
    Open {
        strip: use_consume::<OpenTabs>().0,
        docs: use_consume::<OpenDocs>().0,
    }
}

/// The project's states as a component sees them: through the contexts the root provides,
/// so a view that switches projects needs none of them handed down to it.
pub(crate) fn use_project_states() -> ProjectStates {
    ProjectStates {
        proj: use_consume::<Proj>().0,
        objects: use_consume::<Objects>().0,
        loading: use_consume::<Loading>().0,
        open: use_open(),
        asm_at: use_consume::<AsmAt>().0,
        code_at: use_consume::<CodeAt>().0,
        src_at: use_consume::<SrcAt>().0,
        driven: use_consume::<Drives>().0,
        marks_at: use_consume::<MarksAt>().0,
        visits: use_consume::<Visited>().0,
        bookmarks: use_consume::<Bookmarked>().0,
        searched: use_consume::<Searching>().0,
        build: use_consume::<Building>().0,
        arranged: use_arrangement(),
    }
}

/// The window's arrangement, out of the three contexts it is kept in.
pub(crate) fn use_arrangement() -> Arrangement {
    Arrangement {
        dock: use_consume::<SidebarDock>().0,
        sidebar: use_consume::<SidebarWidth>().0,
        split: use_consume::<SplitRatio>().0,
    }
}

/// The flattened symbol list, shared through context so the Symbols tab does not have to
/// rebuild it.
#[derive(Clone, Copy)]
pub(crate) struct Symbols(pub(crate) Memo<SymbolList>);

/// Every object's text symbols flattened into one list, rebuilt only when the object
/// list changes. Compared by pointer so passing it around stays O(1).
#[derive(Clone)]
pub(crate) struct SymbolList(pub(crate) Arc<Vec<Symbol>>);

impl PartialEq for SymbolList {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

//! The app's root state: every context provided once at the root of `app()` and read with
//! `use_consume` wherever it is wanted, together in one file because they are provided
//! together and a reader asking "what does the app hold" is asking about the set rather
//! than about any one of them.
//!
//! Two of the names here are **derivations and not states**, and that is the distinction
//! the file exists to keep visible: `Active` is a `Memo` over the dock and the document
//! table -- the active document is that panel's active tab read through `Docs`, so there
//! is no second list and no cursor -- and `Symbols` is a `Memo` over `Objects`, every
//! object's text symbols flattened once per change of the list. The two hooks at the end
//! are what provide the rest, in the groups a project switch closes and reopens.

use super::*;

/// The loaded objects, shared through context.
#[derive(Clone, Copy)]
pub(crate) struct Objects(pub(crate) State<Vec<Arc<Object>>>);

/// The files being read into [`Objects`] right now, shared through context so the sidebar
/// can say so.
///
/// It is a state of its own and not a field of the objects list because it is about
/// exactly what that list has *not* got: a file appears here when it is asked for and
/// leaves when nothing more is coming out of it, whether or not it produced anything at
/// all. See [`Loads`] for the model and [`open_binaries`] for what fills it in.
#[derive(Clone, Copy)]
pub(crate) struct Loading(pub(crate) State<Loads>);

/// The active document, shared through context.
///
/// Since 6c this *is* the active tab: everything on screen in the content area is the one
/// entry of [`Open`] that this names. Nothing beside it says which tab is active, and
/// since Step 1 there is nothing beside it saying which *file* is shown either — that was
/// `Shown`, the Source pane's own answer to the same question for its own strip, and the
/// two strips are now one.
///
/// `None` is nothing open *and* a view being the tab on top -- see below.
///
/// **A derivation and not a state.** Since documents became dock tabs there is nothing to
/// keep in step: the active document is the document panel's active tab, read through
/// [`Docs`], and [`active_document`] is that sentence. A `State` beside it would be a
/// second answer to a question the dock already answers, which is the thing
/// [`Open`] exists to prevent.
///
/// A [`Memo`] and not a bare read of the two states, because the dock notifies on every
/// layout change: a reader dragging a split would otherwise re-render every pane that
/// draws the active document. `Memo` writes with `set_if_modified`, so a drag that did not
/// change which document is active wakes nothing.
///
/// **It is therefore a beat behind**, a memo being recomputed by a task that wakes on a
/// notify rather than at the write. That is right for anything that *renders* and wrong
/// for anything that has to be true inside one event handler, which is why the three
/// functions holding the invariants call [`active_document`] on the states directly
/// instead of reading this.
///
/// `None` means two different things and deliberately does not distinguish them: nothing
/// is open, or the tab on top of the document panel is a view. Making Settings the active
/// tab therefore means there is no active document, which is what keeps this a derivation
/// -- the alternative is remembering the last document that was active there, which is
/// memory rather than a reading of the dock.
#[derive(Clone, Copy)]
pub(crate) struct Active(pub(crate) Memo<Option<Document>>);

/// What is open: the panel the reader's document tabs are in, and the table saying what
/// each of those tabs stands for.
///
/// **One source of truth, in two states that cannot disagree.** The panel's `tabs` vec
/// *is* the list of open documents, in the reader's own order -- there is no second list.
/// [`Docs`] holds no order at all, only the mapping a dock tab id needs; membership is the
/// one thing the two share, and the three functions below are what keep it true: a
/// document's tab and its table entry are made together and closed together.
///
/// A plain `Copy` bundle rather than a context of its own, because the two are always
/// wanted together and every one of the functions holding the invariants needs both.
#[derive(Clone, Copy)]
pub(crate) struct Open {
    pub(super) dock: State<DockArea>,
    pub(crate) docs: State<Docs>,
}

/// Every open document, in the order the reader's tabs are in.
///
/// Views are skipped: they are tabs in the same panel but they are not documents, and
/// this is what the session writes down and what a closing binary is walked against.
pub(super) fn open_documents(dock: &DockArea, docs: &Docs) -> Vec<Document> {
    let Some(panel) = dock.document_panel() else {
        return Vec::new();
    };
    panel
        .tabs
        .iter()
        .filter_map(|tab| match tab {
            Tab::Document(id) => docs.get(*id).cloned(),
            Tab::View(_) => None,
        })
        .collect()
}

/// The active document: the document panel's active tab, when that tab is a document.
///
/// The whole of [`Active`], and the reason it needs no memory. `None` for a view on top
/// is deliberate -- see [`Active`].
pub(super) fn active_document(dock: &DockArea, docs: &Docs) -> Option<Document> {
    match dock.document_panel()?.active_tab_id? {
        Tab::Document(id) => docs.get(id).cloned(),
        Tab::View(_) => None,
    }
}

impl Open {
    /// The active document as of *now*, for the event handlers that cannot wait a beat
    /// for [`Active`] to catch up. `peek`, so asking does not subscribe anything.
    pub(crate) fn active(&self) -> Option<Document> {
        let (dock, docs) = (self.dock.peek(), self.docs.peek());
        active_document(&dock, &docs)
    }

    /// Every open document as of now, in tab order. `peek`, for the same reason.
    pub(crate) fn documents(&self) -> Vec<Document> {
        let (dock, docs) = (self.dock.peek(), self.docs.peek());
        open_documents(&dock, &docs)
    }
}

/// The content area's dock, shared through context because the panel documents live in is
/// part of what a project has open rather than only part of the layout.
#[derive(Clone, Copy)]
pub(crate) struct ContentDock(pub(super) State<DockArea>);

/// How wide the assembly side of a document is, as a percentage, and the shared
/// `ResizableContext` it is read back out of.
///
/// **One number for the app rather than one per document**, and it has to be held out here
/// because "the container remembers it" is not on offer. Only the active tab's content is
/// mounted, so a document's split is torn down on every switch of document; a
/// `ResizablePanel` registers itself at its `initial_size` in a `use_hook` and *removes*
/// its entry in a `use_drop`, so even a `ResizableContext` shared through
/// `.controller(..)` comes back holding the initial sizes and a pair of brand-new panel
/// ids. What does survive is a value the app keeps: fed in as `initial_size`, which is
/// exactly what a remount reads, and written back out of the shared context while the
/// split is on screen.
///
/// Per-document would be a third [`Positions`]-shaped map to forget in `close_tab`, for a
/// number nobody has asked to differ per document.
#[derive(Clone, Copy)]
pub(crate) struct SplitRatio(pub(crate) State<f32>);

/// The `ResizableContext` the document's two panels register into, so a drag on the handle
/// can be read back out. See [`SplitRatio`].
#[derive(Clone, Copy)]
pub(crate) struct Splits(pub(crate) State<ResizableContext>);

/// What the assembly side starts at, before the reader has dragged anything.
pub(crate) const DEFAULT_SPLIT: f32 = 50.0;

/// Which row each open tab's **assembly** side was left on, shared through context.
///
/// Beside [`Open`] rather than inside it, and beside it rather than inside
/// [`InstructionList`], for one reason each. Inside `Tabs` it would be a field of what
/// the strip draws, so a scroll of the reader's would re-render every tab; inside the
/// pane it would live and die with the component, which is precisely the bug this fixes —
/// one scroll controller is reused for every symbol, so a tab switch used to leave the
/// new function at the offset the old one was at. Here it outlives both the component and
/// any one document, which is what a *tab's* position has to do.
///
/// Keyed by [`Document`] — the same identity [`Open`] keys by, so an entry means "this
/// tab" for exactly as long as that tab is in the list, and never accidentally means a
/// second symbol of the same name in another object. It is also why the persisted form
/// cannot reuse the key and identifies its tabs by path and name instead (`project.rs`).
#[derive(Clone, Copy)]
pub(crate) struct AsmAt(pub(crate) State<Positions<Document>>);

/// The documents the dock's tabs are handles into, and nothing about their order.
///
/// See [`Docs`]: it exists because a dock tab id must be `Copy + Hash` and a [`Document`]
/// is neither. The order the reader put their tabs in is the document panel's own list,
/// so this is deliberately a table and not a second copy of it.
#[derive(Clone, Copy)]
pub(crate) struct OpenDocs(pub(crate) State<Docs>);

/// Which row each open tab's **source** side was left on. [`AsmAt`]'s other half, and
/// keyed by the same document rather than by the file the pane happens to be showing:
/// a tab has two sides and each remembers its own row, so two functions compiled from one
/// file no longer share a position they have no reason to share.
#[derive(Clone, Copy)]
pub(crate) struct SrcAt(pub(crate) State<Positions<Document>>);

/// Where the reader has been, shared through context. Named `Hist` because `History` is
/// the type it holds, the same way `Active` holds a `Document`.
#[derive(Clone, Copy)]
pub(crate) struct Hist(pub(crate) State<History>);

/// The project the app is in, as the project view holds it.
///
/// Two of its three fields are `String`s where [`Details`] has `Option`s, because this is
/// what is in two text boxes and a text box has no third state: an empty box *is* how a
/// reader says "I have not said". [`OpenProject::details`] is the conversion and is the
/// one place the two spellings meet, so nothing else in the app has to know that an
/// unnamed project is an absent key rather than an empty string.
///
/// This is a state and not a value read out of `project.rs` on demand for the reason
/// every other context here is one: something renders it, so a change to it has to
/// re-render that something. Making it a state is also what let `Saves::given` stop being
/// a value carried across the save calls and become an ordinary baseline — a rename is
/// now a state change like any other, seen by the same observer, and written at once
/// because `name` lives in `project.toml`.
#[derive(Clone, Default, PartialEq)]
pub(crate) struct OpenProject {
    /// The directory the project is stored in, which is its identity. `None` until a
    /// project exists on disk at all — a run in which nothing has been opened or named
    /// has allocated none, deliberately.
    pub(crate) id: Option<ProjectId>,
    pub(crate) name: String,
    pub(crate) directory: String,
}

impl OpenProject {
    /// The project as it was found on disk.
    pub(crate) fn opened(id: ProjectId, project: &Project) -> OpenProject {
        OpenProject {
            id: Some(id),
            name: project.name.clone().unwrap_or_default(),
            directory: project
                .directory
                .as_ref()
                .map(|directory| directory.to_string_lossy().into_owned())
                .unwrap_or_default(),
        }
    }

    /// What of this reaches `project.toml`.
    ///
    /// A box holding nothing but spaces is a box holding nothing: the alternative is a
    /// project named `" "`, which is anonymous everywhere it is drawn and named
    /// everywhere it is compared. Trimmed rather than refused, so the reader is never
    /// told off for a trailing space.
    pub(crate) fn details(&self) -> Details {
        Details {
            name: given(&self.name).map(str::to_owned),
            directory: given(&self.directory).map(PathBuf::from),
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

/// The user's settings as the settings page has them.
///
/// [`OpenProject`]'s shape, and for its reason: `Settings` spells a family the reader has
/// not chosen as an *absent* key, and a text box has no third state -- an empty box **is**
/// how a reader says "I have not said". So the family is a `String` here and an
/// `Option<String>` there, [`EditedSettings::settings`] is the one place the two spellings
/// meet, and it trims, so a box of spaces is a box of nothing rather than a font family
/// named `" "`.
///
/// The size does *not* get the same treatment, and that is the one place this differs.
/// It is edited by a stepper rather than by a text box (see [`SettingsTab`]), so there is
/// no half-typed state to hold and no third answer for text that is not a number: an
/// `Option<f32>` here is an `Option<f32>` there and the mapping is the identity. The
/// theme is likewise the stored enum itself -- three buttons, three answers.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct EditedSettings {
    pub(crate) theme: ThemeChoice,
    pub(crate) interface: EditedFont,
    pub(crate) fixed: EditedFont,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct EditedFont {
    pub(crate) family: String,
    /// In points, like the file and like [`Font::points`], so that the number on screen,
    /// the number the desktop answered and the number written down are one number.
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
    /// screen. Total, deliberately: there is no state of this struct that does not say
    /// something, so nothing between the page and the file can be pending or invalid.
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

/// The settings, shared through context.
///
/// A root context and not state inside the settings view, for the reason `Proj` is one:
/// the page is a dockable tab that may not be mounted, while the theme and the fonts are
/// resolved at the root on every render. The page edits this; [`use_settings`] is what
/// notices.
#[derive(Clone, Copy)]
pub(crate) struct Prefs(pub(crate) State<EditedSettings>);

/// Every state a project owns.
///
/// One value because a project switch touches all of them at once — it closes everything
/// that belonged to the project being left and restores everything that belongs to the
/// one being entered — and because the two halves of that, [`clear_project`] and
/// [`restore_project`], would otherwise be eight-argument functions called from three
/// places. It is `Copy` and holds nothing but handles, so passing it is passing eight
/// pointers.
#[derive(Clone, Copy)]
pub(crate) struct ProjectStates {
    pub(crate) proj: State<OpenProject>,
    pub(crate) objects: State<Vec<Arc<Object>>>,
    /// The files on their way into `objects`. It belongs to the project for the reason
    /// the objects do: leaving one abandons what was being read for it, including the
    /// files that have produced nothing yet and so are not in `objects` to be closed one
    /// by one.
    pub(crate) loading: State<Loads>,
    /// The document panel and the id table: what is open, and in what order. Two states
    /// where there used to be a list and an active document, and one fewer answer to
    /// "which document is on screen" -- see [`Open`] and [`Active`].
    pub(crate) open: Open,
    pub(crate) asm_at: State<Positions<Document>>,
    pub(crate) src_at: State<Positions<Document>>,
    pub(crate) history: State<History>,
}

/// What is open, as a component sees it: the document panel and the id table together.
pub(crate) fn use_open() -> Open {
    Open {
        dock: use_consume::<ContentDock>().0,
        docs: use_consume::<OpenDocs>().0,
    }
}

/// The seven states as a component sees them: through the contexts the root provides, so
/// a view that switches projects needs none of them handed down to it.
pub(crate) fn use_project_states() -> ProjectStates {
    ProjectStates {
        proj: use_consume::<Proj>().0,
        objects: use_consume::<Objects>().0,
        loading: use_consume::<Loading>().0,
        open: use_open(),
        asm_at: use_consume::<AsmAt>().0,
        src_at: use_consume::<SrcAt>().0,
        history: use_consume::<Hist>().0,
    }
}

/// The flattened symbol list, shared through context so the Symbols tab does not
/// have to rebuild it and the root does not have to re-render to hand it over.
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

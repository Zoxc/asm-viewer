//! The app's root state: every context provided once at the root of `app()` and read with
//! `use_consume` wherever it is wanted.
//!
//! Two of the names are **derivations and not states**: `Active` is a `Memo` over the dock
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

/// The active document, shared through context.
///
/// **A derivation and not a state**: the document panel's active tab, read through
/// [`Docs`] -- see [`active_document`]. `None` means both "nothing is open" and "the tab
/// on top is a view", and deliberately does not distinguish them.
///
/// A [`Memo`] because the dock notifies on every layout change, and a drag that changed no
/// document must wake nothing. **It is therefore a beat behind**, which is right for
/// anything that renders and wrong for anything that must be true inside one event
/// handler -- so the functions holding the invariants call [`active_document`] on the
/// states directly instead of reading this.
#[derive(Clone, Copy)]
pub(crate) struct Active(pub(crate) Memo<Option<Document>>);

/// What is open: the panel the reader's document tabs are in, and the table saying what
/// each of those tabs stands for.
///
/// The panel's `tabs` vec *is* the list of open documents, in the reader's own order;
/// [`Docs`] holds no order, only the mapping a dock tab id needs. Membership is the one
/// thing the two share, and `activate`/`close_tab`/`close_binary` keep it true: a
/// document's tab and its table entry are made together and closed together.
#[derive(Clone, Copy)]
pub(crate) struct Open {
    pub(crate) dock: State<DockArea>,
    pub(crate) docs: State<Docs>,
}

/// Every open document, in the order the reader's tabs are in. Views are skipped: they are
/// tabs in the same panel but they are not documents.
pub(crate) fn open_documents(dock: &DockArea, docs: &Docs) -> Vec<Document> {
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
pub(crate) fn active_document(dock: &DockArea, docs: &Docs) -> Option<Document> {
    match dock.document_panel()?.active_tab_id? {
        Tab::Document(id) => docs.get(id).cloned(),
        Tab::View(_) => None,
    }
}

impl Open {
    /// The active document as of *now*, for the event handlers that cannot wait a beat
    /// for [`Active`] to catch up. `peek`, so asking subscribes nothing.
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

/// The content area's dock.
#[derive(Clone, Copy)]
pub(crate) struct ContentDock(pub(crate) State<DockArea>);

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

/// Which row each open tab's **assembly** side was left on. At the root rather than in the
/// pane, which reuses one scroll controller for every symbol and so would leave a newly
/// opened function at the offset the old one was at. Keyed by [`Document`], so an entry
/// means "this tab" for exactly as long as that tab is in the list.
#[derive(Clone, Copy)]
pub(crate) struct AsmAt(pub(crate) State<Positions<Document>>);

/// The documents the dock's tabs are handles into, and nothing about their order. See
/// [`Docs`]: it exists because a dock tab id must be `Copy + Hash` and a [`Document`] is
/// neither.
#[derive(Clone, Copy)]
pub(crate) struct OpenDocs(pub(crate) State<Docs>);

/// Which row each open tab's **source** side was left on. [`AsmAt`]'s other half, keyed by
/// the same document rather than by the file the pane happens to be showing.
#[derive(Clone, Copy)]
pub(crate) struct SrcAt(pub(crate) State<Positions<Document>>);

/// Which source line each source-driven tab's assembly side is driven from, shared
/// through context. Beside [`AsmAt`]/[`SrcAt`] because it is the same kind of thing: a
/// fact about a tab, made by a click in it and forgotten with it.
#[derive(Clone, Copy)]
pub(crate) struct Drives(pub(crate) State<Driven>);

/// Where the reader has been.
#[derive(Clone, Copy)]
pub(crate) struct Hist(pub(crate) State<History>);

/// The project the app is in, as the project view holds it.
///
/// Two of its three fields are `String`s where [`Details`] has `Option`s, because this is
/// what is in two text boxes and a text box has no third state: an empty box *is* how a
/// reader says "I have not said". [`OpenProject::details`] is the one place the two
/// spellings meet.
#[derive(Clone, Default, PartialEq)]
pub(crate) struct OpenProject {
    /// The directory the project is stored in, which is its identity. `None` until a
    /// project exists on disk at all.
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

    /// What of this reaches `project.toml`. Trimmed, so a box holding nothing but spaces
    /// is a box holding nothing.
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
/// view, which is a dockable tab that may not be mounted. The page edits this;
/// `use_settings_with` is what notices.
#[derive(Clone, Copy)]
pub(crate) struct Prefs(pub(crate) State<EditedSettings>);

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
    /// The document panel and the id table: what is open, and in what order.
    pub(crate) open: Open,
    pub(crate) asm_at: State<Positions<Document>>,
    pub(crate) src_at: State<Positions<Document>>,
    /// Which line each source-driven tab's assembly side is driven from.
    pub(crate) driven: State<Driven>,
    pub(crate) history: State<History>,
}

/// What is open, as a component sees it: the document panel and the id table together.
pub(crate) fn use_open() -> Open {
    Open {
        dock: use_consume::<ContentDock>().0,
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
        src_at: use_consume::<SrcAt>().0,
        driven: use_consume::<Drives>().0,
        history: use_consume::<Hist>().0,
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

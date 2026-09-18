//! Live state into a session, and a session back into live state.
//!
//! Out is [`Session::from_state`]: every open tab, where each place was left, and the
//! digest of every binary. Back is [`Session::restore`]: every saved place looked up in
//! the objects loaded now, under one answer about which of their files have been rebuilt
//! since ([`Loaded`]).

use std::{
    collections::{hash_map, BTreeMap, HashMap, HashSet},
    path::{Path, PathBuf},
    sync::Arc,
};

use analysis::{Object, Symbol, SymbolData};

use crate::docs::{DocId, Entry};
use crate::document::Document;
use crate::history::{History, Stop};
use crate::positions::{Driven, Positions, Spot};
use crate::tabs::Page;
use crate::visits::Visits;

use super::files::{
    SavedDocument, SavedEntry, SavedHistory, SavedName, SavedShown, SavedTab, SavedUi, Session,
    SessionCargo,
};

/// The first object out of each file the loaded objects came from, in the order the files
/// were opened, each with how many objects came out of that file: one walk, where a path
/// already seen names the row to count against. [`binaries`], [`binary_counts`] and
/// [`digests`] each read their answer off it.
fn by_file(objects: &[Arc<Object>]) -> Vec<(&Arc<Object>, usize)> {
    let mut files: Vec<(&Arc<Object>, usize)> = Vec::new();
    let mut at: HashMap<&Path, usize> = HashMap::new();
    for object in objects {
        match at.entry(&object.path) {
            hash_map::Entry::Occupied(seen) => files[*seen.get()].1 += 1,
            hash_map::Entry::Vacant(unseen) => {
                unseen.insert(files.len());
                files.push((object, 1));
            }
        }
    }
    files
}

/// Every binary the loaded objects came out of, deduplicated, in the order they were
/// opened — which is [`super::Project::binaries`], derived rather than tracked.
pub fn binaries(objects: &[Arc<Object>]) -> Vec<PathBuf> {
    by_file(objects)
        .into_iter()
        .map(|(object, _)| object.path.clone())
        .collect()
}

/// The same binaries, each with how many of the loaded objects came out of it: what the
/// Project view lists. Counted on the one walk, a filter per binary being a walk each.
pub fn binary_counts(objects: &[Arc<Object>]) -> Vec<(PathBuf, usize)> {
    by_file(objects)
        .into_iter()
        .map(|(object, count)| (object.path.clone(), count))
        .collect()
}

/// The digest of every binary those objects came out of, keyed by the same path
/// [`binaries`] keys them by: what [`Session::digests`] is written from.
///
/// Read off the object rather than computed here: the hash was taken once, on the parse
/// worker thread, and every object out of one file answers the same thing — so an
/// archive's members cost one pass rather than one each.
fn digests(objects: &[Arc<Object>]) -> BTreeMap<PathBuf, String> {
    by_file(objects)
        .into_iter()
        .map(|(object, _)| (object.path.clone(), object.data.digest().to_string()))
        .collect()
}

/// One tab as the app holds it, on its way into a [`SavedTab`]: the bar's order is the
/// order of these, and a document's trail is borrowed rather than cloned.
pub enum SavingTab<'a> {
    Page(Page),
    Document {
        id: DocId,
        trail: &'a History,
        temporal: bool,
    },
}

/// What was on screen when the session was written. Three answers and not an
/// `Option<&Document>`: a page is on screen, a document is, or nothing is.
#[derive(Clone, Copy)]
pub enum OnScreen<'a> {
    Nothing,
    Page(Page),
    Document(&'a Document),
}

/// Where every open place was left, as [`Session::from_state`] is handed it: the four
/// maps a saved entry's two rows, driven line and scrolled address come out of, each
/// keyed by the tab and the place it belongs to.
///
/// One bundle rather than four arguments handed down, since a saved place wants all
/// four and wants them keyed the same way. Built by the caller, and by name: three of the
/// four are `&Positions` of near-identical type, so a field name says which is which
/// where a position could not.
pub struct LeftAt<'a> {
    pub asm_rows: &'a Positions<Entry>,
    pub src_rows: &'a Positions<Entry>,
    pub places: &'a Positions<Entry, Spot>,
    pub driven: &'a Driven,
}

/// What the app noticed that is not a place, as [`Session::from_state`] is handed it: the
/// agreement to run a language server, what the last build produced, and how the window
/// was arranged.
pub struct Noticed<'a> {
    pub trusted: bool,
    pub artifacts: &'a [PathBuf],
    pub ui: SavedUi,
}

impl LeftAt<'_> {
    /// One open tab as it is saved: a page is its name and nothing else; a document is
    /// its whole trail, with the cursor on the place it showed and whether it was the
    /// temporal tab.
    fn tab(&self, tab: &SavingTab<'_>) -> SavedTab {
        match tab {
            SavingTab::Page(page) => SavedTab {
                page: Some(page.stored().to_owned()),
                temporal: false,
                cursor: 0,
                entries: Vec::new(),
            },
            SavingTab::Document {
                id,
                trail,
                temporal,
            } => SavedTab {
                page: None,
                temporal: *temporal,
                cursor: trail.cursor().unwrap_or(0),
                entries: trail
                    .entries()
                    .iter()
                    .map(|stop| self.entry(*id, stop))
                    .collect(),
            },
        }
    }

    /// One place on a tab's trail as it is saved: the place itself, and where each of its
    /// two sides was left. A side that was never scrolled has no entry in its
    /// [`Positions`] at all and is written out as row `0`.
    fn entry(&self, id: DocId, stop: &Stop) -> SavedEntry {
        let entry = (id, stop.clone());
        SavedEntry {
            asm_row: self.asm_rows.at(&entry).unwrap_or(0),
            src_row: self.src_rows.at(&entry).unwrap_or(0),
            line: self.driven.line(&entry),
            asm_address: self.places.at(&entry).map(|spot| spot.address),
            code_address: stop.address(),
            src_line: stop.line(),
            document: SavedDocument::from_document(&stop.document),
        }
    }
}

impl SavedHistory {
    fn from_visits(visits: &Visits) -> SavedHistory {
        SavedHistory {
            entries: visits
                .entries()
                .iter()
                .map(SavedDocument::from_document)
                .collect(),
        }
    }
}

/// The binaries that are no longer the files the session was saved against.
///
/// A mismatch is not an error and not a refusal to open; it only decides how much of a
/// saved place may still be believed. The **name** is believed, since a rebuild keeps
/// most of its function names. The **address** is not: under a rebuilt file it stops
/// being a requirement (recovering a symbol that merely moved) and stops being evidence
/// (a name that names two symbols and no longer names an address resolves to neither).
/// The saved **row** is not either, being a claim about a listing this build no longer
/// has.
#[derive(Debug)]
enum Changed {
    /// The paths whose saved digest no longer matches the file loaded under them.
    Paths(HashSet<PathBuf>),
    /// Every path, whatever the digests say: what a bookmark resolves under
    /// ([`SavedDocument::resolve_by_name`]), being saved against no digest at all.
    Every,
}

/// What a saved place is resolved against: the objects loaded now, and which of the files
/// they came out of have changed since the session was saved.
///
/// A restore resolves a few hundred places against one object list — every entry of every
/// tab, up to 200 visits, and the active document — so the list is indexed once and each
/// place is a lookup in it. A scan per place is a component-wise `Path` compare against
/// every member of an archive that can hold thousands.
struct Loaded<'a> {
    objects: Lookup<'a>,
    changed: Changed,
}

/// Where the object a saved place names is found.
enum Lookup<'a> {
    /// Indexed by the file and member name a saved place names an object by: what a
    /// restore builds, one pass for the many lookups that follow.
    Index(HashMap<(&'a Path, &'a str), &'a Arc<Object>>),
    /// The list itself, scanned. What a **single** lookup uses, since building the index
    /// would cost more than the one scan it saves: [`SavedDocument::resolve_by_name`],
    /// asked per bookmark and again per drawn row.
    Scan(&'a [Arc<Object>]),
}

impl<'a> Loaded<'a> {
    /// The loaded objects indexed, and every saved digest compared against the file
    /// loaded under that path now. Only a digest present on both sides and *different* is
    /// a rebuild. Per saved path rather than per object, so an archive's 196 members ask
    /// it once.
    fn of(session: &Session, objects: &'a [Arc<Object>]) -> Loaded<'a> {
        let mut index = HashMap::with_capacity(objects.len());
        let mut first: HashMap<&Path, &Arc<Object>> = HashMap::new();
        for object in objects {
            // First in the list wins both, which is where a scan of it stopped.
            index
                .entry((object.path.as_path(), object.name.as_str()))
                .or_insert(object);
            first.entry(object.path.as_path()).or_insert(object);
        }
        let mut changed = HashSet::new();
        for (path, digest) in &session.digests {
            let Some(object) = first.get(path.as_path()) else {
                continue;
            };
            if object.data.digest().to_string() != *digest {
                log::debug!(
                    "{} has changed since the session was saved; matching by name",
                    path.display()
                );
                changed.insert(path.clone());
            }
        }
        Loaded {
            objects: Lookup::Index(index),
            changed: Changed::Paths(changed),
        }
    }

    /// The objects scanned rather than indexed, every file taken as changed: what one
    /// place resolved on its own is resolved against ([`SavedDocument::resolve_by_name`]).
    fn scanning(objects: &'a [Arc<Object>]) -> Loaded<'a> {
        Loaded {
            objects: Lookup::Scan(objects),
            changed: Changed::Every,
        }
    }

    /// The loaded object a saved place names, if it is still there.
    fn object(&self, saved: &SavedDocument) -> Option<Arc<Object>> {
        let (path, name) = saved.binary()?;
        match &self.objects {
            Lookup::Index(index) => index.get(&(path, name)).map(|object| (*object).clone()),
            Lookup::Scan(objects) => objects
                .iter()
                .find(|object| object.path == path && object.name == name)
                .cloned(),
        }
    }

    fn changed(&self, path: &Path) -> bool {
        match &self.changed {
            Changed::Paths(paths) => paths.contains(path),
            Changed::Every => true,
        }
    }
}

/// Everything a restore puts back, which [`Session::restore`] answers in one call.
pub struct Restored {
    /// The record of visits.
    pub visits: Visits,
    /// The tabs that still resolve, in the order the bar was in.
    pub tabs: Vec<RestoredTab>,
    /// The document that was on screen, degraded rather than dropped.
    pub active: Option<Document>,
}

/// One tab a restore opens: a page, or a document with something left on its trail. What
/// [`Restored::tabs`] holds, in the order the bar was in.
///
/// A document's trail is live, its cursor carried past the entries that no longer
/// resolve; `entries` holds the rows of every place still on it, in the trail's own
/// order, newest place first.
#[derive(Clone, PartialEq)]
pub enum RestoredTab {
    Page(Page),
    Document {
        temporal: bool,
        trail: History,
        entries: Vec<RestoredEntry>,
    },
}

/// One place of a restored tab that still points somewhere, with the rows its two sides
/// were left at.
///
/// Named rather than a tuple because it is seven things, and because the rows and the
/// two addresses drop under a rebuilt binary while the two lines do not.
#[derive(Clone, PartialEq)]
pub struct RestoredEntry {
    pub document: Document,
    pub asm_row: usize,
    pub src_row: usize,
    pub line: Option<u32>,
    /// The address an object's code tab was scrolled to, and nothing else.
    pub address: Option<u64>,
    /// The address the place itself is at, where it is a place in an object's code.
    pub code_address: Option<u64>,
    /// The line the place itself is, where it is a place in a source file.
    pub src_line: Option<u32>,
}

impl RestoredEntry {
    /// The place this is, as a trail holds one ([`Stop`]).
    ///
    /// The halves are paired back with the document by [`Stop::paired`], which is where
    /// that rule lives: a file states them apart and can therefore state a pairing that
    /// means nothing. Past here nothing carries the halves.
    pub fn stop(&self) -> Stop {
        Stop::paired(self.document.clone(), self.code_address, self.src_line)
    }
}

impl SavedDocument {
    /// The saved form of `document`.
    pub fn from_document(document: &Document) -> SavedDocument {
        match document {
            Document::Code(object) => SavedDocument::Object {
                path: object.path.clone(),
                object_name: object.name.clone(),
                shown: SavedShown::Code,
            },
            Document::Object(object) => SavedDocument::Object {
                path: object.path.clone(),
                object_name: object.name.clone(),
                shown: SavedShown::Symbols,
            },
            Document::Symbol(symbol) => SavedDocument::Symbol {
                path: symbol.object.path.clone(),
                object_name: symbol.object.name.clone(),
                address: symbol.data.address,
                symbol_name: SavedName::of(&symbol.data.name, symbol.data.address),
            },
            Document::Source(file) => SavedDocument::Source {
                path: file.to_string(),
            },
        }
    }

    /// Exactly what this names, or `None` when the object — or, for a symbol, the symbol
    /// — is no longer loaded. What history entries want: an entry that no longer points
    /// where it did is dropped rather than turned into a destination the user never
    /// visited.
    ///
    /// A source-driven entry resolves against nothing and so cannot fail: a deleted file
    /// comes back as a tab over the pane's own "Source file not found".
    fn resolve(&self, loaded: &Loaded) -> Option<Document> {
        match self {
            SavedDocument::Source { path } => Some(Document::Source(Arc::from(path.as_str()))),
            SavedDocument::Object { shown, .. } => {
                let object = loaded.object(self)?;
                Some(match shown {
                    SavedShown::Symbols => Document::Object(object),
                    SavedShown::Code => Document::Code(object),
                })
            }
            SavedDocument::Symbol {
                path,
                symbol_name,
                address,
                ..
            } => {
                let object = loaded.object(self)?;
                let data = SavedDocument::find_symbol(
                    &object,
                    &symbol_name.text(*address),
                    *address,
                    loaded.changed(path),
                )?
                .clone();
                Some(Document::Symbol(Symbol { object, data }))
            }
        }
    }

    /// What this names against whatever is loaded, believing the **name** over the address
    /// whether or not the file is known to have changed: [`Changed::Every`]'s reading of
    /// [`SavedDocument::resolve`]. What a bookmark resolves by.
    ///
    /// It is the answer the digest-aware rule gives wherever the two could be compared. An
    /// unchanged file still holds the exact name-and-address pair the place was saved
    /// with, so the exact match wins there as it does under the strict rule; a rebuilt file
    /// is read exactly as a rebuilt file is. And a bookmark cannot be read the other way:
    /// `Session::digests` is the digest at the last *session* save, not at the bookmark's
    /// making, so on the second launch after a rebuild the file would read as unchanged and
    /// a stale address would drop a bookmark the reader made on purpose.
    pub fn resolve_by_name(&self, objects: &[Arc<Object>]) -> Option<Document> {
        self.resolve(&Loaded::scanning(objects))
    }

    /// The symbol a saved place names, under a file that either is or is not the one it
    /// was saved against.
    ///
    /// The candidates are the run of `symbols_sorted` that carries the name, found by two
    /// binary searches over a list that is sorted by name — 115k entries on the repo's own
    /// binary.
    ///
    /// **Unchanged** (or never hashed): the name *and* the address, which is what tells two
    /// same-named symbols apart.
    ///
    /// **Rebuilt**: the name, with the address as a tie-breaker only. An exact match is
    /// still preferred; failing that, a name that names exactly one symbol resolves to
    /// it. A name that names several and matches no address resolves to **nothing** —
    /// picking one on the strength of a stale address is how a reader ends up on a
    /// function they never opened.
    fn find_symbol<'a>(
        object: &'a Object,
        name: &str,
        address: u64,
        rebuilt: bool,
    ) -> Option<&'a Arc<SymbolData>> {
        let sorted = &object.symbols_sorted;
        let from = sorted.partition_point(|data| data.name.as_str() < name);
        let named = &sorted[from..];
        let named = &named[..named.partition_point(|data| data.name == name)];

        let exact = named.iter().find(|data| data.address == address);
        match (exact, rebuilt, named) {
            (Some(data), _, _) => Some(data),
            (None, true, [one]) => Some(one),
            (None, _, _) => None,
        }
    }

    /// The same, degrading instead of failing: a symbol that is gone falls back to its
    /// object and an object that is gone to nothing at all. What the *active document*
    /// wants, there being one of it and the app having to open somewhere.
    fn resolve_or_degrade(&self, loaded: &Loaded) -> Option<Document> {
        self.resolve(loaded)
            .or_else(|| loaded.object(self).map(Document::Object))
    }
}

impl SavedTab {
    /// This tab against the objects that are now loaded, or [`None`] where nothing of it
    /// is left: a page this build does not have, or a document whose every place has
    /// gone.
    ///
    /// A place that no longer resolves is **dropped** from the trail rather than
    /// degraded, the cursor carried the way [`History::rebuilt`] carries it -- the same
    /// walk closing a file goes through, so the two cannot drift.
    fn restore(&self, loaded: &Loaded) -> Option<RestoredTab> {
        // A page resolves against nothing, and one this build does not have is dropped as
        // a place that no longer resolves is.
        if let Some(page) = &self.page {
            return Some(RestoredTab::Page(Page::from_stored(page)?));
        }
        let resolved: Vec<Option<RestoredEntry>> = self
            .entries
            .iter()
            .map(|entry| entry.restore(loaded))
            .collect();
        let trail = History::rebuilt(
            resolved
                .iter()
                .map(|entry| entry.as_ref().map(RestoredEntry::stop)),
            self.cursor,
        );
        // A tab with nothing left on its trail is dropped: a strip whose tabs all
        // degraded onto the same object would collapse into one.
        trail.current()?;
        // The rows of what survived, in the trail's order; `rebuilt` keeps the survivors
        // in the order they were given, so the two agree.
        let entries = resolved.into_iter().flatten().collect();
        Some(RestoredTab::Document {
            temporal: self.temporal,
            trail,
            entries,
        })
    }
}

impl SavedEntry {
    /// This place against the objects that are now loaded, or [`None`] where it no longer
    /// names one.
    ///
    /// A row is a claim about a listing, so a **rebuilt** listing takes both its rows with
    /// it, and an address is one too: the scroll and the place's own address both go, so a
    /// place in an object's code comes back as the whole listing. A file has no binary
    /// path and so is never rebuilt. The two lines are claims about a *file* rather than
    /// about a listing, so they survive a rebuild and are simply asked again.
    fn restore(&self, loaded: &Loaded) -> Option<RestoredEntry> {
        let document = self.document.resolve(loaded)?;
        let changed = self
            .document
            .binary_path()
            .is_some_and(|path| loaded.changed(path));
        let (asm_row, src_row, address, code_address) = match changed {
            true => (0, 0, None, None),
            false => (
                self.asm_row,
                self.src_row,
                self.asm_address,
                self.code_address,
            ),
        };
        Some(RestoredEntry {
            document,
            asm_row,
            src_row,
            line: self.line,
            address,
            code_address,
            src_line: self.src_line,
        })
    }
}

impl Session {
    /// The session described by the state the app is currently in — the one place the
    /// app's state is turned into what would be saved, [`binaries`] being the other half
    /// of it for the other file. `tabs` is each open tab in strip order: its id, its
    /// trail, and whether it is the temporal one; the four maps under it are where each
    /// place was left ([`LeftAt`]), and the rest of what the app noticed is [`Noticed`].
    pub fn from_state(
        objects: &[Arc<Object>],
        tabs: &[SavingTab<'_>],
        left: &LeftAt<'_>,
        shown: OnScreen<'_>,
        visits: &Visits,
        noticed: Noticed<'_>,
    ) -> Session {
        let Noticed {
            trusted,
            artifacts,
            ui,
        } = noticed;
        Session {
            // Absent here and stamped by [`super::saves::Saves::record`]: which project this is belongs
            // to the save policy, not to the state the app is in.
            id: None,
            active_page: match shown {
                OnScreen::Page(page) => Some(page.stored().to_owned()),
                OnScreen::Document(_) | OnScreen::Nothing => None,
            },
            trusted,
            // Absent rather than empty, so a window nobody has arranged writes no section.
            ui: (ui != SavedUi::default()).then_some(ui),
            // Absent rather than empty, so a project nothing was ever built in writes no
            // section at all.
            cargo: (!artifacts.is_empty()).then(|| SessionCargo {
                artifacts: artifacts.to_vec(),
            }),
            digests: digests(objects),
            active: match shown {
                OnScreen::Document(document) => Some(SavedDocument::from_document(document)),
                OnScreen::Page(_) | OnScreen::Nothing => None,
            },
            tabs: tabs.iter().map(|tab| left.tab(tab)).collect(),
            history: SavedHistory::from_visits(visits),
        }
    }

    /// The record of visits, the tabs and the active document against the objects that
    /// are now loaded.
    ///
    /// **One call, because the three are one question.** They share a [`Loaded`] -- the
    /// objects indexed once, and one walk of the saved digests against them -- so a tab
    /// and the active document cannot be resolved under two different answers about which
    /// binaries have changed, and a caller cannot take one and forget the others.
    /// [`Session::pages`] and [`Session::shown_page`] stay outside it: they resolve
    /// against no object and go back before any binary has been read.
    pub fn restore(&self, objects: &[Arc<Object>]) -> Restored {
        let loaded = Loaded::of(self, objects);
        Restored {
            visits: self.resolve_history(&loaded),
            tabs: self.resolve_tabs(&loaded),
            active: self.resolve_active(&loaded),
        }
    }

    /// The saved active document against the objects that are now loaded. Degrades
    /// silently: a symbol that is gone falls back to its object, an object that is gone
    /// to nothing.
    fn resolve_active(&self, loaded: &Loaded) -> Option<Document> {
        let saved = self.active.as_ref()?;
        saved.resolve_or_degrade(loaded)
    }

    /// The page that was on screen, where one was and this build still has it.
    pub fn shown_page(&self) -> Option<Page> {
        Page::from_stored(self.active_page.as_deref()?)
    }

    /// The saved pages with the place each had in the bar. A page resolves against no
    /// object, so these are what a restore can put back before any binary has been read
    /// -- and all it has to put back for a project with no binaries at all.
    pub fn pages(&self) -> impl Iterator<Item = (usize, Page)> + '_ {
        self.tabs.iter().enumerate().filter_map(|(position, tab)| {
            Some((position, Page::from_stored(tab.page.as_deref()?)?))
        })
    }

    /// The saved tabs as live trails, in strip order, each place with the rows its two
    /// sides were left at. A tab with nothing left of it ([`SavedTab::restore`]) is
    /// dropped, and the tabs that survive keep their order.
    fn resolve_tabs(&self, loaded: &Loaded) -> Vec<RestoredTab> {
        self.tabs
            .iter()
            .filter_map(|saved| saved.restore(loaded))
            .collect()
    }

    /// The saved record of visits as a live one. A place that no longer resolves is
    /// dropped: a list of places the reader cannot get back to is worse than a short
    /// list.
    fn resolve_history(&self, loaded: &Loaded) -> Visits {
        Visits::restored(
            self.history
                .entries
                .iter()
                .filter_map(|saved| saved.resolve(loaded))
                .collect(),
        )
    }
}

#[cfg(test)]
mod tests;

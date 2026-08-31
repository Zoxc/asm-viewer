//! DWARF line-number information, read lazily out of the bytes an [`Object`] was parsed
//! from.
//!
//! Nothing here runs at parse time. The first caller that asks a given object for line
//! info pays for building a [`Dwarf`] context; every later question is answered from it.
//! An object with no DWARF — the sample `LLVM-24-rust-dev.dll` is a PE whose debug info,
//! if it had any, would be CodeView in a PDB, and the sample `.rlib`'s member is COFF with
//! `.debug$S`/`.debug$T` — caches *that* answer too, so it is asked once and never again.
//!
//! ## Lifetimes
//!
//! `addr2line::Context<R>` borrows nothing from an `object::File`; it borrows through its
//! `gimli::Reader`. Handing it readers that borrow [`ObjectData`](crate::ObjectData) would
//! make [`Object`] self-referential, which is the trap this module exists to avoid: the
//! readers are [`gimli::EndianArcSlice`] instead, i.e. `Arc<[u8]>` per DWARF section, so
//! the context owns its data outright and is `'static`. Those `Arc`s are *not* slices of
//! `Object::data` — a section may be compressed, and every section is relocated in place
//! (see [`relocate`]) — so each is a separate allocation, sized by the `.debug_*` sections
//! and nothing else.

use crate::{section_data, Object, SymbolData};
use gimli::{EndianArcSlice, RunTimeEndian};
use object::{Object as _, ObjectSection, ObjectSymbol, RelocationKind, RelocationTarget};
use std::collections::HashMap;
use std::ops::Range;
use std::sync::{Arc, Mutex, OnceLock};

/// Every DWARF section is read through one of these: an `Arc<[u8]>` plus an endianness.
/// Owning the bytes rather than borrowing them is what keeps [`Dwarf`] free of any
/// lifetime, and cloning a reader — which `gimli` does constantly — is an `Arc` bump.
type Reader = EndianArcSlice<RunTimeEndian>;

/// An [`Object`]'s DWARF, or the fact that it has none, worked out at most once.
///
/// Caching the *absence* matters as much as caching the context: without it, every query
/// against an object with no debug info — the common case for a stripped binary, and for
/// both of the repo's sample files — would re-parse its section table looking for
/// `.debug_info` that is not there.
///
/// A field of [`Object`] rather than something a caller builds; the only thing to do with
/// one is `DwarfCache::default()`.
#[derive(Default)]
pub struct DwarfCache(OnceLock<Option<Dwarf>>);

/// One object's DWARF, parsed once and kept for the object's lifetime.
///
/// The [`Mutex`] is not about contention, it is about `Sync`. `addr2line::Context` caches
/// each unit's parsed line program in an `UnsafeCell` behind `&self` (`addr2line::lazy`),
/// so it is `Send` but deliberately not `Sync` — upstream's own test only asserts `Send`.
/// [`Object`] is shared as an `Arc` across threads, so the context has to be behind a lock
/// to travel with it. Contention is a non-issue in practice: a query is a binary search
/// plus a walk of a handful of line-table rows, and the one call that does real work (the
/// first query into a compilation unit, which parses its line program) is exactly the one
/// that must not be repeated anyway.
pub(crate) struct Dwarf {
    context: Mutex<addr2line::Context<Reader>>,
}

impl Dwarf {
    /// Build the context for one object file, or [`None`] when it has no DWARF to build
    /// one from. Never an error: foreign debug info and corrupt debug info are both
    /// simply "no line info".
    pub(crate) fn load(data: &crate::ObjectData) -> Option<Dwarf> {
        without_panicking(|| Dwarf::load_inner(data)).flatten()
    }

    fn load_inner(data: &crate::ObjectData) -> Option<Dwarf> {
        let file = object::File::parse(data.bytes()).ok()?;

        // The cheap test first, so an object with no DWARF costs one section-table scan
        // and not a single byte of parsing. `section_by_name` already knows about ELF's
        // `.zdebug_info` and Mach-O's `__debug_info` spellings.
        file.section_by_name(gimli::SectionId::DebugInfo.name())?;

        let endian = if file.is_little_endian() {
            RunTimeEndian::Little
        } else {
            RunTimeEndian::Big
        };

        let dwarf = gimli::Dwarf::load::<_, ()>(|id| Ok(load_section(&file, id, endian))).ok()?;

        Some(Dwarf {
            context: Mutex::new(addr2line::Context::from_dwarf(dwarf).ok()?),
        })
    }

    /// Resolve a whole address range in one pass; see [`Object::line_info`].
    fn line_info(&self, range: Range<u64>) -> Option<Arc<LineInfo>> {
        without_panicking(|| self.line_info_inner(range)).flatten()
    }

    fn line_info_inner(&self, range: Range<u64>) -> Option<Arc<LineInfo>> {
        // A poisoned lock means a previous query panicked. Nothing here is left in a
        // half-written state by a panic (the context is only ever read), so recover
        // rather than propagating: "no line info" would be a worse answer than the
        // right one, and a panic is what the tests exist to keep from happening.
        let context = self.context.lock().unwrap_or_else(|e| e.into_inner());

        let mut files: Vec<Arc<str>> = Vec::new();
        let mut file_indices: HashMap<Arc<str>, usize> = HashMap::new();
        let mut rows: Vec<LineRow> = Vec::new();

        // One call for the symbol's whole extent rather than one per instruction: this
        // walks each covering unit's line table once, where per-instruction lookups
        // would binary-search it again for every address.
        for (address, length, location) in context.find_location_range(range.start, range.end).ok()?
        {
            let Some(end) = address.checked_add(length) else {
                continue;
            };
            // addr2line hands back the row *containing* `range.start`, which may begin
            // before it, and clips nothing at the top either.
            let start = address.max(range.start);
            let end = end.min(range.end);
            if start >= end {
                continue;
            }

            let file = location.file.map(|file| match file_indices.get(file) {
                Some(index) => *index,
                None => {
                    let file: Arc<str> = Arc::from(file);
                    files.push(file.clone());
                    file_indices.insert(file, files.len() - 1);
                    files.len() - 1
                }
            });

            rows.push(LineRow {
                range: start..end,
                file,
                line: location.line,
                column: location.column,
            });
        }

        drop(context);

        // Units are visited in range order and rows within a unit ascend, but two units
        // may cover overlapping addresses, so sort rather than assume.
        rows.sort_by_key(|row| (row.range.start, row.range.end));

        // Coalesce runs that say the same thing. A line program emits a row per
        // is_stmt/discriminator change as well as per source position, so a single
        // source line routinely arrives as several adjacent identical rows.
        rows.dedup_by(|next, row| {
            let same = row.range.end == next.range.start
                && row.file == next.file
                && row.line == next.line
                && row.column == next.column;
            if same {
                row.range.end = next.range.end;
            }
            same
        });

        // "There is DWARF but it says nothing about this symbol" and "there is no DWARF"
        // are the same answer to a caller, so give it the same shape.
        (!rows.is_empty()).then(|| Arc::new(LineInfo { rows, files }))
    }
}

/// Run DWARF parsing with a net under it, turning a panic into "no line info".
///
/// This is not defensiveness in general — it is one known, reachable bug. `addr2line`
/// 0.21 computes a line-table row's length as `next.address - row.address` with a plain
/// subtraction (`LocationRangeUnitIter::next`), and nothing stops a line program from
/// moving its address *backwards*: `DW_LNE_set_address` takes any address, and a sequence
/// whose end address is below its last row is enough on its own. On a debug build — which
/// is how this app is run while it is being developed — that is an "attempt to subtract
/// with overflow" panic on a file the user merely opened, and this crate's one hard rule
/// is that no file input makes it fall over. `crates/analysis/tests/robustness.rs`
/// (`a_line_program_that_runs_backwards_does_not_panic`) is that input, reduced.
///
/// Catching is sound here because a panic leaves nothing half-written: the context is
/// only ever read, `addr2line`'s internal caches are filled after the value is computed
/// rather than during, and the lock the panic poisons is recovered explicitly. On a
/// release build the subtraction wraps instead of panicking and the absurd length it
/// produces is clipped away by the range check in [`Dwarf::line_info_inner`], so the
/// guard changes nothing there.
///
/// The panic message still reaches stderr through the process-wide hook; suppressing it
/// would mean installing a global hook, which a library has no business doing.
fn without_panicking<T>(f: impl FnOnce() -> T) -> Option<T> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).ok()
}

/// Read one DWARF section, decompressing and relocating it. A section that is missing or
/// unreadable becomes an empty reader, which is what `gimli` expects for "not present".
fn load_section(file: &object::File<'_>, id: gimli::SectionId, endian: RunTimeEndian) -> Reader {
    let data = file
        .section_by_name(id.name())
        .and_then(|section| {
            // The same guard the rest of the crate uses: a section header that lies
            // about how much it decompresses to is dropped, not believed. `.debug_*`
            // sections are the ones that are actually compressed in practice, so this
            // path needs it at least as much as the text sections do.
            let mut data = section_data(&section)?;
            relocate(&mut data, file, &section, endian);
            Some(data)
        })
        .unwrap_or_default();

    EndianArcSlice::new(Arc::from(data), endian)
}

/// Apply a debug section's relocations to a copy of its bytes.
///
/// In a relocatable object the addresses in `.debug_info` and `.debug_line` are not in
/// the file: `DW_AT_low_pc` and `DW_LNE_set_address` are written as zero with a
/// relocation against the function's symbol, so line info read without relocating maps
/// every function in the object to address 0. Since the bytes are already a private copy,
/// they can be patched in place — no `gimli::Reader` wrapper is needed the way
/// `dwarfdump`'s is.
///
/// Only `Absolute` relocations are applied, which is what DWARF's address and offset
/// forms use. Anything else (a COFF `SECREL`, say) is left alone rather than guessed at,
/// so a section keeps whatever the file already held there.
///
/// The value written is `symbol/section address + addend`, plus the bytes already there
/// when the format keeps the addend in the section (ELF `REL`, COFF) rather than in the
/// relocation (ELF `RELA`). Every step wraps and every write is bounds-checked, so no
/// relocation table, however corrupt, can do more than scribble on this copy.
fn relocate<'data, 'file>(
    data: &mut [u8],
    file: &object::File<'data>,
    section: &object::Section<'data, 'file>,
    endian: RunTimeEndian,
) {
    for (offset, relocation) in section.relocations() {
        if relocation.kind() != RelocationKind::Absolute {
            continue;
        }

        let target = match relocation.target() {
            RelocationTarget::Symbol(index) => file.symbol_by_index(index).ok().map(|s| s.address()),
            RelocationTarget::Section(index) => {
                file.section_by_index(index).ok().map(|s| s.address())
            }
            _ => None,
        };
        let Some(target) = target else { continue };

        let Ok(offset) = usize::try_from(offset) else {
            continue;
        };
        let size = usize::from(relocation.size()) / 8;
        let Some(bytes) = data.get_mut(offset..offset.wrapping_add(size)) else {
            continue;
        };

        let implicit = if relocation.has_implicit_addend() {
            read_uint(bytes, endian)
        } else {
            0
        };
        let value = implicit
            .wrapping_add(target)
            .wrapping_add(relocation.addend() as u64);

        write_uint(bytes, endian, value);
    }
}

/// The 4- or 8-byte unsigned at `bytes`. Any other width is not a DWARF address or
/// offset and is left to the caller to ignore.
fn read_uint(bytes: &[u8], endian: RunTimeEndian) -> u64 {
    let mut buffer = [0u8; 8];
    match (bytes.len(), endian) {
        (4, RunTimeEndian::Little) | (8, RunTimeEndian::Little) => {
            buffer[..bytes.len()].copy_from_slice(bytes);
            u64::from_le_bytes(buffer)
        }
        (4, _) | (8, _) => {
            buffer[8 - bytes.len()..].copy_from_slice(bytes);
            u64::from_be_bytes(buffer)
        }
        _ => 0,
    }
}

/// The inverse of [`read_uint`]; a width it does not understand is left untouched.
fn write_uint(bytes: &mut [u8], endian: RunTimeEndian, value: u64) {
    let len = bytes.len();
    match (len, endian) {
        (4, RunTimeEndian::Little) | (8, RunTimeEndian::Little) => {
            bytes.copy_from_slice(&value.to_le_bytes()[..len]);
        }
        (4, _) | (8, _) => {
            bytes.copy_from_slice(&value.to_be_bytes()[8 - len..]);
        }
        _ => {}
    }
}

/// One run of instructions and the source position DWARF gives it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LineRow {
    /// The instruction addresses this row covers, clipped to the range that was asked
    /// about and in the same address space as [`SymbolData::address`] and
    /// [`Instruction::address`](crate::Instruction::address).
    pub range: Range<u64>,
    /// An index into [`LineInfo::files`], or [`None`] when the row names no file.
    pub file: Option<usize>,
    /// DWARF's line number. Genuinely optional: a row with line 0 says "these
    /// instructions belong to no source line", which is not the same as line 0 and not
    /// the same as line 1.
    pub line: Option<u32>,
    /// DWARF's column number, [`None`] both when the producer emitted no column at all
    /// (the common case outside clang) and when it emitted `DW_LNS_set_column 0`, the
    /// "left edge of the line" marker.
    pub column: Option<u32>,
}

/// A source position, for callers that want one address answered rather than a row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Location<'a> {
    pub file: Option<&'a str>,
    pub line: Option<u32>,
    pub column: Option<u32>,
}

/// The line info covering one address range, resolved in a single pass.
///
/// This is the shape the assembly view wants: it holds a `Vec<Instruction>` for a symbol
/// and needs a source position for each, so it asks once for the symbol and then answers
/// each instruction locally with [`row_at`](Self::row_at) — a binary search over rows that
/// are typically an order of magnitude fewer than the instructions. Asking per instruction
/// instead would re-enter the DWARF context, and its lock, once per row of the view.
///
/// The rows are ascending, non-overlapping, and coalesced: adjacent rows that name the
/// same position are merged, so a source line that produced twenty instructions is one
/// row and not twenty. They are *not* contiguous — compiler-generated instructions
/// belonging to no source line leave gaps, and [`row_at`](Self::row_at) returns [`None`]
/// there rather than inventing a position.
pub struct LineInfo {
    rows: Vec<LineRow>,
    files: Vec<Arc<str>>,
}

impl LineInfo {
    /// Every row, ascending by address.
    pub fn rows(&self) -> &[LineRow] {
        &self.rows
    }

    /// The set of source files these rows touch, deduplicated, in the order they were
    /// first seen. [`LineRow::file`] indexes into this.
    pub fn files(&self) -> &[Arc<str>] {
        &self.files
    }

    /// The row covering `address`, or [`None`] when no row does.
    pub fn row_at(&self, address: u64) -> Option<&LineRow> {
        let index = match self
            .rows
            .binary_search_by_key(&address, |row| row.range.start)
        {
            Ok(index) => index,
            Err(0) => return None,
            Err(index) => index - 1,
        };
        let row = &self.rows[index];
        (address < row.range.end).then_some(row)
    }

    /// The file a row names.
    pub fn file_of(&self, row: &LineRow) -> Option<&str> {
        Some(&*self.files[row.file?])
    }

    /// `(file, line, column)` for a single instruction address.
    pub fn location(&self, address: u64) -> Option<Location<'_>> {
        let row = self.row_at(address)?;
        Some(Location {
            file: self.file_of(row),
            line: row.line,
            column: row.column,
        })
    }
}

impl Object {
    /// The line info for an address range, building this object's DWARF context on the
    /// first call and reusing it afterwards.
    ///
    /// [`None`] means "no line info": no DWARF, debug info in a format this does not read
    /// (PE + CodeView, COFF `.debug$S`), DWARF that will not parse, or DWARF that simply
    /// says nothing about this range. Never an error — an object the app can list is an
    /// object it can show, with or without sources.
    ///
    /// **Cost.** The first call parses `.debug_abbrev` and every compilation unit's
    /// header and range list; later calls do not. Each call then parses the line program
    /// of every unit covering the range, once per unit for the object's lifetime. This is
    /// worker-thread work by construction — call it where [`SymbolData::assembly`] is
    /// called, not on a UI thread.
    pub fn line_info(&self, range: Range<u64>) -> Option<Arc<LineInfo>> {
        self.dwarf()?.line_info(range)
    }

    /// This object's DWARF context, built at most once — including the "there is none"
    /// answer, so an object without debug info is not re-examined on every query.
    fn dwarf(&self) -> Option<&Dwarf> {
        self.dwarf
            .0
            .get_or_init(|| Dwarf::load(&self.data))
            .as_ref()
    }
}

impl SymbolData {
    /// The line info for this symbol's instructions, over the extent
    /// [`estimate_size`](Self::estimate_size) derives — the same bytes
    /// [`assembly`](Self::assembly) decodes, so every instruction it produces is inside
    /// the range asked about here.
    ///
    /// Takes the object for the same reason [`assembly`](Self::assembly) does: a symbol
    /// does not own the file it came from.
    pub fn line_info(&self, object: &Object) -> Option<Arc<LineInfo>> {
        let end = self.address.checked_add(self.estimate_size()?)?;
        object.line_info(self.address..end)
    }
}

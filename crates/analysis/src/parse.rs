//! One object file read into an [`Object`]: its sections and where each is placed, its
//! symbols, the code it declares outside its symbol table, and the names demangled.

use crate::demangle;
use crate::line::{DebugInfo, Declared};
use crate::sections::{bias_of, section_biases, section_data, Placement};
use crate::unwind::{self, UnwindEntry};
use crate::{
    Import, LoadMessage, MadeUp, Object, ObjectData, PlacedAddress, Section, SectionAddress,
    SymbolData,
};
use object::macho;
use object::read::macho::{MachHeader, MachOFile};
use object::{
    Architecture, BigEndian, BinaryFormat, Endian, Endianness, ExportTarget, FileFlags,
    Object as _, ObjectKind, ObjectSection, ObjectSegment, ObjectSymbol, ReadRef, RelocationTarget,
    SectionIndex, SectionKind, SymbolIndex, SymbolKind, SymbolSection,
};
use std::{
    cell::Cell,
    collections::{BTreeMap, HashMap, HashSet},
    ops::Range,
    path::PathBuf,
    sync::Arc,
};

/// A symbol's name as the parse holds it, until [`symbol_data`] builds the symbol. Only a
/// [`Name::Symbol`] is offered to the demanglers.
pub(crate) enum Name {
    /// The file's own spelling, or a debug file's decorated one: offered to the demanglers.
    Symbol(String),
    /// A debug file's name that is already fit to show, which no demangler has anything to
    /// say about.
    Informative(String),
    /// One of ours, rendered to a `String` only when the symbol is built, which keeps it
    /// too ([`SymbolData::made_up`]).
    MadeUp(MadeUp),
}

/// One symbol as the file states it, in its symbol table or elsewhere ([`declared_code`]),
/// held until the whole object's names are demangled in one batch ([`symbol_data`]).
struct Pending {
    index: SymbolIndex,
    name: Name,
    address: SectionAddress,
    /// What the file said: a symbol's size, the length a debug file's record states, or an
    /// unwind entry's stated end less its begin. [`None`] for an export, the entry point, a
    /// record with no length of its own, and a symbol whose size field is 0 ([`stated`]).
    /// The extent used comes from [`SymbolData::extent`], which reads this only where the
    /// format makes it a function's length ([`SymbolData::declared_extent`]).
    size: Option<u64>,
    /// The section the symbol is in, looked up when the symbol is built. For declared code it
    /// is the code section containing `address`: an export table and an entry point name an
    /// address and nothing else.
    section: Option<SectionIndex>,
}

/// The text symbols of a file's symbol table, out of [`symbol_table`].
struct SymbolTable {
    /// Each one whose name reads, in table order.
    named: Vec<Pending>,
    /// Each one whose name will not read, in table order, called by its address. One claims
    /// its placed address only where nothing else named it (the table, [`declared_code`], or
    /// one of these before it), so an export, a name out of the debug file, or an unwind
    /// entry can still give it a real name, and a name at the same offset of another section
    /// of a relocatable object does not drop it.
    unnamed: Vec<Pending>,
    /// Each undefined one whose name reads, in table order: an import, with no code here.
    imports: Vec<Import>,
    /// The first index past the table, which declared code is numbered from.
    next: usize,
}

/// The code a file declares outside its symbol table: its **entry point**, its **exports**,
/// its ELF `.dynsym` (whose undefined functions go to `imports`), the functions its **debug
/// file** names where there is one (`named`, out of [`DebugInfo::declared`]), and the
/// **unwind entries** of an x86-64 PE's exception directory or an ELF's `.eh_frame`
/// (`unwind`, out of [`unwind::entries`]). A stripped shared library is otherwise a file with
/// nothing in it, and a `/DEBUG` image has no symbol table at all.
/// Every address here is one the file — or the debug file matched to it by GUID and age —
/// states outright, so the "nothing is scanned for" rule still holds. A function's and the
/// entry point's are read through to the code first, where the format tags them or points
/// at a descriptor ([`CodeAddresses`]).
///
/// Three decisions the caller depends on:
///
/// **Only in a code section.** An address is looked up in the kept [`SectionKind::Text`]
/// sections and that section becomes the symbol's own; it doubles as the filter keeping
/// exported *data* out.
///
/// **One symbol per address, earliest source winning** (symbol table > dynamic symbol >
/// export > entry point > debug file > unwind entry). An export is very often the symbol
/// table's own function under its exported name, and a second `SymbolData` for it would be a
/// second row in the list for one place in the file. The debug file comes after the image so
/// a name the image itself states is never displaced by the debug file's spelling of it, and
/// its own records are taken in the order it hands them over, which is the order it wants
/// them believed. The unwind entries come last of all because they carry no name: one at an
/// address anything else named adds nothing, and one nothing named is called
/// `<function 0x…>` by its address — or `<fragment 0x…>` where its unwind info is chained, a
/// second range of some function's rather than a function ([`UnwindEntry`]).
///
/// **Nothing for a relocatable object.** `entry()` answers 0 for an `.o`, and 0 there is a
/// real function's first byte.
///
/// The indices start at `next`, *past* the file's own symbol table, which is the only honest
/// thing they can be. Nothing can reach them by relocation, since a file that declares
/// exports is a linked image.
#[allow(clippy::too_many_arguments)]
fn declared_code(
    file: &object::File<'_>,
    addresses: &CodeAddresses<'_, '_>,
    code: &[(Range<SectionAddress>, SectionIndex)],
    known: &mut HashSet<PlacedAddress>,
    imports: &mut Vec<Import>,
    next: usize,
    named: Vec<Declared>,
    unwind: &[UnwindEntry],
) -> Vec<Pending> {
    let mut declared = Vec::new();
    if file.kind() == ObjectKind::Relocatable {
        return declared;
    }

    // Which kind of name it is decides whether it is offered to the demanglers.
    let mut take = |name: Name, address: SectionAddress, size: Option<u64>| {
        let Some((_, section)) = code.iter().find(|(range, _)| range.contains(&address)) else {
            return;
        };
        // `known` is keyed by placed address, and nothing here is a relocatable object's
        // ([`section_biases`]), so nothing placed this one.
        if !known.insert(address.unplaced()) {
            return;
        }
        declared.push(Pending {
            index: SymbolIndex(next + declared.len()),
            name,
            address,
            size,
            section: Some(*section),
        });
    };

    // An import the symbol table already named is not listed twice.
    let mut imported: HashSet<String> = imports.iter().map(|i| i.name.clone()).collect();
    // The stated addresses of the dynamic functions whose code is somewhere else.
    let mut moved = HashSet::new();
    for symbol in file.dynamic_symbols() {
        if symbol.kind() != SymbolKind::Text {
            continue;
        }
        let Ok(name) = symbol.name_bytes() else {
            continue;
        };
        if name.is_empty() {
            continue;
        }
        if symbol.is_undefined() {
            let name = String::from_utf8_lossy(name).into_owned();
            if imported.insert(name.clone()) {
                imports.push(import(name, symbol.address()));
            }
            continue;
        }
        let stated_at = Code {
            address: symbol.address(),
            section: symbol.section().index(),
            size: stated(symbol.size()),
        };
        let Some(code) = addresses.function(file, stated_at) else {
            continue;
        };
        if code.address != symbol.address() {
            moved.insert(symbol.address());
        }
        take(
            Name::Symbol(String::from_utf8_lossy(name).into_owned()),
            SectionAddress::new(code.address),
            code.size,
        );
    }

    // `exports` reports one entry at a time, so a malformed one is skipped rather than
    // taken as the end of the table. An export names a place in this image only when it
    // has a name and an address: one identified by ordinal has nothing to draw, and a
    // forwarder or a re-export names a place in another image. The name is the file's, and
    // on a Windows DLL very often MSVC-mangled.
    for export in file.exports().into_iter().flatten().flatten() {
        let ExportTarget::Address { address } = export.target() else {
            continue;
        };
        // An ELF's exports are its `.dynsym` again. One whose code the walk above found
        // elsewhere is that function, and taken as stated it would be a byte into it
        // (Thumb) or its descriptor.
        if moved.contains(&address) {
            continue;
        }
        let Some(name) = export.name().into_name() else {
            continue;
        };
        if name.is_empty() {
            continue;
        }
        take(
            Name::Symbol(String::from_utf8_lossy(name).into_owned()),
            SectionAddress::new(address),
            None,
        );
    }

    // 0 is "this image has no entry point", which is how a DLL built without one states it.
    let entry = match file {
        object::File::MachO32(file) => macho_entry(file),
        object::File::MachO64(file) => macho_entry(file),
        _ => Some(file.entry()),
    };
    let entry = entry
        .filter(|&entry| entry != 0)
        .and_then(|entry| addresses.entry(file, entry));
    if let Some(entry) = entry {
        take(
            Name::MadeUp(MadeUp::EntryPoint),
            SectionAddress::new(entry),
            None,
        );
    }

    // After the image's own names, so they win, and among themselves in the order the debug
    // file handed them over. The addresses are already in the image's space, and the
    // code-section lookup is what drops a record the debug file places in a section the
    // image does not have code in — a public the linker flagged as code that is not, say.
    for function in named {
        take(function.name, function.address, function.len);
    }

    // Last of all, the unwind entries: an address and a length for whatever is still
    // unnamed, and no name at all.
    for entry in unwind {
        take(
            Name::MadeUp(MadeUp::unwind(entry)),
            entry.range.start,
            Some(entry.len()),
        );
    }

    declared
}

/// A Mach-O's entry point as an address, from the first `LC_MAIN` or `LC_UNIXTHREAD` whose
/// entry can be read, as in `object`'s own walk. Not through `entry()`, which answers
/// `LC_MAIN`'s `entryoff`, a file offset, as it is (`notes/upstream/object.md`). That offset
/// is placed through the segment whose file bytes hold it, and is no entry point when none
/// does. An `LC_UNIXTHREAD`'s PC is already an address ([`thread_pc`]).
fn macho_entry<'data, Mach: MachHeader, R: ReadRef<'data>>(
    file: &MachOFile<'data, Mach, R>,
) -> Option<u64> {
    let endian = file.endian();
    let mut commands = file.macho_load_commands().ok()?;
    while let Ok(Some(command)) = commands.next() {
        if let Ok(Some(main)) = command.entry_point() {
            let offset = main.entryoff.get(endian);
            let (segment, into) = file.segments().find_map(|segment| {
                let (start, size) = segment.file_range();
                let into = offset.checked_sub(start).filter(|&into| into < size)?;
                Some((segment, into))
            })?;
            return segment.address().checked_add(into);
        }
        if let Ok(Some((_, state))) = command.unix_thread() {
            let cputype = file.macho_header().cputype(endian);
            if let Some(pc) = thread_pc(endian, cputype, state) {
                return Some(pc);
            }
        }
    }
    None
}

/// The PC in an `LC_UNIXTHREAD`'s thread state, at the place `object` 0.40 reads it from:
/// past the flavor and the count, then after the registers each CPU puts before it. [`None`]
/// for any other CPU or a state too short to hold it.
fn thread_pc<E: Endian>(endian: E, cputype: macho::CpuType, state: &[u8]) -> Option<u64> {
    let (offset, size): (usize, usize) = match cputype {
        // x86_thread_state64: rax to r15, then rip.
        macho::CPU_TYPE_X86_64 => (8 + 16 * 8, 8),
        // arm_thread_state64: x0 to x28, fp, lr, sp, then pc.
        macho::CPU_TYPE_ARM64 => (8 + 32 * 8, 8),
        // x86_thread_state32: ten registers, then eip.
        macho::CPU_TYPE_X86 => (8 + 10 * 4, 4),
        // arm_thread_state32: r0 to r12, sp, lr, then pc.
        macho::CPU_TYPE_ARM => (8 + 15 * 4, 4),
        _ => return None,
    };
    let bytes = state.get(offset..offset.checked_add(size)?)?;
    match size {
        8 => Some(endian.read_u64(bytes.try_into().ok()?)),
        _ => Some(u64::from(endian.read_u32(bytes.try_into().ok()?))),
    }
}

/// A function's code as the parse takes it: where it is, the section it is in, and the size
/// the file states for it.
#[derive(Clone, Copy)]
struct Code {
    address: u64,
    section: Option<SectionIndex>,
    size: Option<u64>,
}

/// How the numbers a file states for its functions and its entry point become the addresses
/// of their code. On most formats they already are. On three, `object` hands them over as
/// the file states them, and they are not (`notes/upstream/object.md`).
struct CodeAddresses<'data, 'file> {
    rule: Rule<'data, 'file>,
    /// How many functions were left out because their descriptor could not be read.
    unread: Cell<usize>,
}

enum Rule<'data, 'file> {
    /// Each number is the code's address.
    Direct,
    /// 32-bit ARM ELF: bit 0 of a function's address is the Thumb flag ([`thumb_code`]).
    Thumb,
    /// PPC64 ELFv1: a function's symbol and the entry point name its descriptor in `.opd`
    /// ([`opd_code`]).
    Opd(Opd<'data, 'file>),
    /// XCOFF: the entry point names its descriptor ([`xcoff_entry`]). Its function symbols
    /// are code already: `object` calls a descriptor csect data.
    Xcoff,
}

/// A PPC64 ELFv1 `.opd`, and in a relocatable object the relocations that fill its
/// descriptors, by their offset into it.
struct Opd<'data, 'file> {
    section: object::Section<'data, 'file>,
    relocations: HashMap<u64, (RelocationTarget, i64)>,
}

impl<'data, 'file> Opd<'data, 'file> {
    fn of(file: &'file object::File<'data>, section: object::Section<'data, 'file>) -> Self {
        let mut relocations = HashMap::new();
        if file.kind() == ObjectKind::Relocatable {
            for (offset, relocation) in section.relocations() {
                relocations
                    .entry(offset)
                    .or_insert((relocation.target(), relocation.addend()));
            }
        }
        Opd {
            section,
            relocations,
        }
    }
}

impl<'data, 'file> CodeAddresses<'data, 'file> {
    fn of(file: &'file object::File<'data>) -> Self {
        let rule = match (file.format(), file.architecture(), file.flags()) {
            (BinaryFormat::Elf, Architecture::Arm, _) => Rule::Thumb,
            // ABI 2 has no descriptors; 0 is "unstated", and has them where there is an `.opd`.
            (BinaryFormat::Elf, Architecture::PowerPc64, FileFlags::Elf { e_flags, .. })
                if e_flags.ppc64_abi() != 2 =>
            {
                match file.section_by_name(".opd") {
                    Some(section) => Rule::Opd(Opd::of(file, section)),
                    None => Rule::Direct,
                }
            }
            (BinaryFormat::Xcoff, _, _) => Rule::Xcoff,
            _ => Rule::Direct,
        };
        CodeAddresses {
            rule,
            unread: Cell::new(0),
        }
    }

    /// A defined function symbol's code, or [`None`] where it names a descriptor that
    /// cannot be read. Only a function's: a data symbol's address is never tagged.
    fn function(&self, file: &object::File<'data>, symbol: Code) -> Option<Code> {
        match &self.rule {
            Rule::Thumb => Some(Code {
                address: thumb_code(symbol.address),
                ..symbol
            }),
            // A symbol outside `.opd` names code already: older toolchains' `.foo`.
            Rule::Opd(opd) if symbol.section == Some(opd.section.index()) => {
                let code = opd_code(file, opd, symbol.address);
                self.count(code.is_none());
                code
            }
            _ => Some(symbol),
        }
    }

    /// The code at the entry point a linked image states, or [`None`] where it names a
    /// descriptor that cannot be read.
    fn entry(&self, file: &object::File<'data>, entry: u64) -> Option<u64> {
        let code = match &self.rule {
            Rule::Thumb => return Some(thumb_code(entry)),
            Rule::Opd(opd) if at(&opd.section, entry).is_some() => {
                opd_code(file, opd, entry).map(|code| code.address)
            }
            Rule::Xcoff => xcoff_entry(file, entry),
            _ => return Some(entry),
        };
        self.count(code.is_none());
        code
    }

    fn count(&self, unread: bool) {
        if unread {
            self.unread.set(self.unread.get().saturating_add(1));
        }
    }

    /// What is said about the functions left out, if any were.
    fn message(&self) -> Option<LoadMessage> {
        let count = self.unread.get();
        (count > 0).then_some(LoadMessage::UnreadableDescriptors { count })
    }
}

/// A 32-bit ARM function's code address. The ARM ELF ABI sets bit 0 of a Thumb function's
/// `st_value`, and of `e_entry`, to say the code is Thumb; the code starts a byte lower.
/// Only `STT_FUNC` has the flag, and the parse takes no other kind: a label (`STT_NOTYPE`),
/// the `$a`/`$t`/`$d` mapping symbols among them, is never a function here.
fn thumb_code(address: u64) -> u64 {
    address & !1
}

/// The code a PPC64 ELFv1 function descriptor at `address` in `.opd` names. The first
/// doubleword of the descriptor, in the file's byte order, is the code's address; the size
/// a descriptor symbol states is the descriptor's, so none is kept. In a relocatable object
/// that doubleword is 0 until the linker writes it, so the relocation that fills it is read
/// instead.
fn opd_code(file: &object::File<'_>, opd: &Opd<'_, '_>, address: u64) -> Option<Code> {
    let offset = at(&opd.section, address)?;
    if file.kind() == ObjectKind::Relocatable {
        let &(target, addend) = opd.relocations.get(&offset)?;
        let (base, section) = match target {
            RelocationTarget::Symbol(index) => {
                let symbol = file.symbol_by_index(index).ok()?;
                (symbol.address(), symbol.section().index())
            }
            RelocationTarget::Section(index) => {
                (file.section_by_index(index).ok()?.address(), Some(index))
            }
            _ => return None,
        };
        return Some(Code {
            address: base.checked_add_signed(addend)?,
            section,
            size: None,
        });
    }
    let endian = Endianness::from_big_endian(!file.is_little_endian())?;
    let code = read_word(&opd.section, offset, 8, endian)?;
    let section = file
        .sections()
        .find(|section| at(section, code).is_some())
        .map(|section| section.index());
    Some(Code {
        address: code,
        section,
        size: None,
    })
}

/// The code an XCOFF entry point names. `o_entry` is the address of the entry's function
/// descriptor, whose first word, 4 or 8 bytes by the file's class and big-endian, is the
/// code's address.
fn xcoff_entry(file: &object::File<'_>, entry: u64) -> Option<u64> {
    let width = if file.is_64() { 8 } else { 4 };
    file.sections().find_map(|section| {
        let offset = at(&section, entry)?;
        read_word(&section, offset, width, BigEndian)
    })
}

/// How far into `section` `address` is, where the section covers it.
fn at(section: &object::Section<'_, '_>, address: u64) -> Option<u64> {
    address
        .checked_sub(section.address())
        .filter(|&offset| offset < section.size())
}

/// The `width`-byte word, 4 or 8, `offset` bytes into `section`'s data.
fn read_word(
    section: &object::Section<'_, '_>,
    offset: u64,
    width: usize,
    endian: impl Endian,
) -> Option<u64> {
    let data = section.data().ok()?;
    let start = usize::try_from(offset).ok()?;
    let bytes = data.get(start..start.checked_add(width)?)?;
    match width {
        8 => Some(endian.read_u64(bytes.try_into().ok()?)),
        _ => Some(u64::from(endian.read_u32(bytes.try_into().ok()?))),
    }
}

/// Older PPC64 ELFv1 toolchains name a function's code `.foo` beside `foo`, its
/// descriptor. Read through the descriptor, `foo` is at the same place, so `.foo` is the
/// same function twice. It is dropped, and its size, which is the code's where `foo`'s
/// was the descriptor's, goes to `foo`.
fn drop_dot_names(named: &mut Vec<Pending>) {
    fn spelled(pending: &Pending) -> Option<&str> {
        match &pending.name {
            Name::Symbol(name) => Some(name),
            _ => None,
        }
    }
    let dotted: HashMap<_, usize> = named
        .iter()
        .enumerate()
        .filter_map(|(index, pending)| {
            let name = spelled(pending)?.strip_prefix('.')?;
            Some(((name, pending.address, pending.section), index))
        })
        .collect();
    let twins: Vec<(usize, usize)> = named
        .iter()
        .enumerate()
        .filter_map(|(index, pending)| {
            let key = (spelled(pending)?, pending.address, pending.section);
            Some((index, *dotted.get(&key)?))
        })
        .collect();

    let mut dropped = HashSet::new();
    for (kept, dot) in twins {
        named[kept].size = named[kept].size.or(named[dot].size);
        dropped.insert(dot);
    }
    let mut index = 0;
    named.retain(|_| {
        let keep = !dropped.contains(&index);
        index += 1;
        keep
    });
}

/// The address ranges code can be in, each with its section: what [`declared_code`] looks a
/// declared address up in, and what places an unwind entry's range in its
/// [`CodeSection::unwind`](crate::CodeSection::unwind). Each is the section's own
/// [`bytes_range`](Section::bytes_range), so only the sections that hold code are here — one
/// whose bytes would not decompress was dropped, having nothing to disassemble either — and
/// only the ones whose bytes have addresses to sit at.
///
/// In the file's own section order, which is what decides the section an address in two
/// overlapping ranges is taken to be in.
fn code_sections(
    sections: &HashMap<SectionIndex, Section>,
) -> Vec<(Range<SectionAddress>, SectionIndex)> {
    let mut ranges: Vec<(Range<SectionAddress>, SectionIndex)> = sections
        .values()
        .filter_map(|section| Some((section.bytes_range()?, section.index)))
        .collect();
    ranges.sort_unstable_by_key(|&(_, index)| index.0);
    ranges
}

/// Parse `data` as a single object file. `name` is the display name (an archive member name
/// or the file name) and `path` the file it came from. Anything that fails to parse yields
/// [`None`]. `data` is kept in the returned [`Object`]; see [`ObjectData`].
pub fn parse_object(data: ObjectData, name: String, path: PathBuf) -> Option<Arc<Object>> {
    let file = object::File::parse(data.bytes()).ok()?;

    let mut messages = Vec::new();
    let sections = read_sections(&file, &mut messages);
    let addresses = CodeAddresses::of(&file);
    let SymbolTable {
        named: mut symbols,
        unnamed,
        mut imports,
        next,
    } = symbol_table(&file, &addresses);
    // Keyed by placed address: in a relocatable object every section starts at 0, so an
    // address alone does not say which code it is ([`section_biases`]).
    let place = |symbol: &Pending| {
        let section = symbol.section.and_then(|index| sections.get(&index));
        section.map_or(symbol.address.unplaced(), |section| {
            section.place(symbol.address)
        })
    };
    let mut known: HashSet<PlacedAddress> = symbols.iter().map(place).collect();

    // The debug file is opened here and not on the first line question, because the
    // functions it names are ones the image itself does not declare. The backend it builds
    // is kept for the line questions later.
    let (preloaded, named) = match DebugInfo::declared(&file, &path) {
        Some((info, named)) => (Some(info), named),
        None => (None, Vec::new()),
    };
    let unwind = unwind::entries(&file);
    let code = code_sections(&sections);
    let declared = declared_code(
        &file,
        &addresses,
        &code,
        &mut known,
        &mut imports,
        next,
        named,
        &unwind,
    );
    messages.extend(addresses.message());
    let ranges = place_unwind(&code, &unwind);

    // After `declared_code`, so `known` holds every address anything named.
    symbols.extend(
        unnamed
            .into_iter()
            .filter(|symbol| known.insert(place(symbol))),
    );
    symbols.extend(declared);

    let sections = freeze_sections(sections, ranges);
    let symbols = symbol_data(symbols, &sections);

    let format = file.format();
    let architecture = file.architecture();
    let mut object = Object::preloaded(
        path,
        name,
        format,
        architecture,
        symbols,
        imports,
        sections.into_values().collect(),
        data,
        preloaded,
    );
    object.messages = messages;
    Some(Arc::new(object))
}

/// Every section the file states, by index. Each code section's place is decided here, once,
/// for the line info and the code listing both ([`section_biases`]), and what went wrong
/// deciding it is pushed onto `messages`.
fn read_sections(
    file: &object::File<'_>,
    messages: &mut Vec<LoadMessage>,
) -> HashMap<SectionIndex, Section> {
    let Placement { biases, message } = section_biases(file);
    messages.extend(message);
    let format = file.format();
    file.sections()
        .filter_map(|section| {
            let index = section.index();
            let name = String::from_utf8_lossy(section.name_bytes().ok()?).into_owned();

            // Only a code section's bytes are read here. Whatever reads another -- the line
            // info, the unwind tables -- reads it out of the file again, so a copy here would
            // be a second one held for the object's life. A code section whose bytes will not
            // decompress is dropped outright: there is nothing to disassemble in it and
            // nothing else to keep it for.
            if section.kind() != SectionKind::Text {
                return Some((
                    index,
                    Section::other(index, name, SectionAddress::new(section.address())),
                ));
            }
            let data = section_data(&section)?;

            // Mach-O states a relocation's place as an offset from the start of its section,
            // and lays its sections out one after another, so that offset is not the address
            // for any section but the first. Every lookup here is by address, so the
            // conversion is done once, where the map is built. ELF and COFF need none: a
            // relocatable object's sections are all at 0, and a linked ELF's `r_offset` is
            // already an address. Only a code section's are collected, because the only
            // reader is the disassembler's operand lookup; the DWARF backend takes a debug
            // section's from the file.
            let base = match format {
                BinaryFormat::MachO => section.address(),
                _ => 0,
            };
            let mut relocations = BTreeMap::<_, Vec<_>>::new();
            for (offset, relocation) in section.relocations() {
                if let Some(address) = SectionAddress::new(base).checked_add(offset) {
                    relocations.entry(address).or_default().push(relocation);
                }
            }
            let bias = bias_of(&biases, Some(index));
            Some((
                index,
                Section::text(
                    index,
                    name,
                    data,
                    SectionAddress::new(section.address()),
                    relocations,
                    bias,
                ),
            ))
        })
        .collect()
}

/// The file's text symbols.
///
/// A symbol whose name will not read is a place in the file all the same. It is set aside
/// until the rest have claimed their addresses ([`SymbolTable::unnamed`]). An undefined one
/// is an import and no place in the file: `object` calls an undefined ELF `STT_FUNC` and a
/// COFF external of function type text too ([`SymbolTable::imports`]). So is a COFF weak
/// external ([`weak_external`]). A defined one is taken at its code ([`CodeAddresses`]),
/// and left out where that is a descriptor that cannot be read.
fn symbol_table(file: &object::File<'_>, addresses: &CodeAddresses<'_, '_>) -> SymbolTable {
    let mut table = SymbolTable {
        named: Vec::new(),
        unnamed: Vec::new(),
        imports: Vec::new(),
        next: 0,
    };
    for symbol in file.symbols() {
        table.next = table.next.max(symbol.index().0 + 1);
        if symbol.kind() != SymbolKind::Text {
            continue;
        }
        if symbol.is_undefined() || weak_external(file, &symbol) {
            if let Ok(name) = symbol.name_bytes() {
                let name = String::from_utf8_lossy(name).into_owned();
                table.imports.push(import(name, symbol.address()));
            }
            continue;
        }

        let stated_at = Code {
            address: symbol.address(),
            section: symbol.section().index(),
            size: stated(symbol.size()),
        };
        let Some(code) = addresses.function(file, stated_at) else {
            continue;
        };
        let address = SectionAddress::new(code.address);

        let pending = |name: Name| Pending {
            index: symbol.index(),
            name,
            address,
            size: code.size,
            section: code.section,
        };
        match symbol.name_bytes() {
            Ok(name) => table.named.push(pending(Name::Symbol(
                String::from_utf8_lossy(name).into_owned(),
            ))),
            Err(_) => table
                .unnamed
                .push(pending(Name::MadeUp(MadeUp::Function(address)))),
        }
    }
    if let Rule::Opd(_) = addresses.rule {
        drop_dot_names(&mut table.named);
    }
    table
}

/// Whether `symbol` is a COFF weak external with no section. `object` calls one of function
/// type text, and neither undefined nor in a section, but the linker binds it to a
/// definition elsewhere or to the default its auxiliary record names: it has no code of its
/// own, and as a symbol it would be a row at address 0.
fn weak_external(file: &object::File<'_>, symbol: &object::Symbol<'_, '_>) -> bool {
    matches!(file.format(), BinaryFormat::Coff | BinaryFormat::Pe)
        && symbol.is_weak()
        && symbol.section() == SymbolSection::Unknown
}

/// An import named `name` at the address a symbol table states for it, where one does.
fn import(name: String, address: u64) -> Import {
    Import {
        name,
        address: (address != 0).then(|| SectionAddress::new(address)),
    }
}

/// A symbol table's size field as a size: a symbol whose field is 0 states none, which is
/// what an ELF `st_size` of 0 means and how `object` answers for a format with no such field.
fn stated(size: u64) -> Option<u64> {
    (size != 0).then_some(size)
}

/// Every unwind entry's range, by the section it starts in, whether or not its begin became a
/// symbol: an export or a procedure at that address takes its extent from the end the entry
/// states. The first section holding the start takes it.
fn place_unwind(
    code: &[(Range<SectionAddress>, SectionIndex)],
    unwind: &[UnwindEntry],
) -> HashMap<SectionIndex, Vec<Range<SectionAddress>>> {
    let mut ranges: HashMap<SectionIndex, Vec<Range<SectionAddress>>> = HashMap::new();
    for UnwindEntry { range, .. } in unwind {
        let Some((_, index)) = code
            .iter()
            .find(|(bounds, _)| bounds.contains(&range.start))
        else {
            continue;
        };
        ranges.entry(*index).or_default().push(range.clone());
    }
    ranges
}

/// The sections, done with: each given the unwind ranges that start in it, then shared.
/// [`Section::with_unwind`] clamps each range's end to the section's bytes, so that end can
/// never reach past what `bytes` can read.
fn freeze_sections(
    sections: HashMap<SectionIndex, Section>,
    mut ranges: HashMap<SectionIndex, Vec<Range<SectionAddress>>>,
) -> HashMap<SectionIndex, Arc<Section>> {
    sections
        .into_iter()
        .map(|(index, section)| {
            let unwind = ranges.remove(&index).unwrap_or_default();
            (index, Arc::new(section.with_unwind(unwind)))
        })
        .collect()
}

/// Every symbol built, by index, with its section looked up and its name demangled.
///
/// One batch for the whole object, on stacks of their own and on as many cores as the pool
/// has; see [`demangle`]. Only the [`Name::Symbol`]s are offered: each is *moved* into the
/// batch and comes back out beside the rest of its symbol, in the order it went in. 115k
/// names is not a copy worth making.
fn symbol_data(
    symbols: Vec<Pending>,
    sections: &HashMap<SectionIndex, Arc<Section>>,
) -> HashMap<SymbolIndex, Arc<SymbolData>> {
    let mut built = HashMap::with_capacity(symbols.len());
    let mut build =
        |index, name, demangled, made_up, address, size, section: Option<SectionIndex>| {
            let section = section.and_then(|index| sections.get(&index).cloned());
            let symbol = SymbolData::parsed(name, demangled, made_up, address, section, size);
            built.insert(index, Arc::new(symbol));
        };

    // `waiting[i]` is the rest of the symbol `offered[i]` names.
    let mut offered = Vec::new();
    let mut waiting = Vec::new();
    for Pending {
        index,
        name,
        address,
        size,
        section,
    } in symbols
    {
        match name {
            Name::Symbol(name) => {
                offered.push(name);
                waiting.push((index, address, size, section));
            }
            Name::Informative(name) => build(index, name, None, None, address, size, section),
            Name::MadeUp(made_up) => build(
                index,
                made_up.to_string(),
                None,
                Some(made_up),
                address,
                size,
                section,
            ),
        }
    }

    let (names, demangled) = demangle::batch(offered);
    for ((index, address, size, section), (name, demangled)) in
        waiting.into_iter().zip(names.into_iter().zip(demangled))
    {
        build(index, name, demangled, None, address, size, section);
    }
    built
}

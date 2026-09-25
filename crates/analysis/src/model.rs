//! The data model: an [`Object`], its [`Section`]s and its symbols, the bytes it was parsed
//! from, and the [`LoadMessage`]s saying what went wrong while it was read. Built by
//! [`parse_object`](crate::parse_object) and read by everything else.
//! Also [`covering`], the one search the crate looks an address up in a sorted list of
//! ranges with, and [`FirstCovering`], the lookup over ranges that may overlap built on it.

use crate::disasm::Code;
use crate::extent::ExtentCache;
use crate::line::{DebugInfo, DebugInfoCache};
use crate::{Assembly, Bias, MadeUp, PlacedAddress, SectionAddress};
use object::{Architecture, BinaryFormat, Endianness, Relocation, SectionIndex, SymbolIndex};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    fmt,
    hash::{Hash, Hasher},
    ops::Range,
    path::PathBuf,
    sync::Arc,
};

pub struct Object {
    pub path: PathBuf,
    pub name: String,
    /// [`None`] for a file that is not an object at all, shown only to say why
    /// ([`messages`](Self::messages)): a file that could not be read, one that is no object
    /// this reader can parse, or an archive with none of its members shown. It has no
    /// sections and no symbols. Which of these is an archive is
    /// [`is_archive`](Self::is_archive). Also `None` for a
    /// [`placeholder`](Self::placeholder), a file not read yet.
    pub format: Option<BinaryFormat>,

    /// The machine the code in here is for, as the file's own header declares it. This is
    /// what picks a disassembler ([`SymbolData::assembly`]) and the only thing that can: a
    /// symbol's bytes say nothing about how to read themselves.
    pub architecture: Architecture,
    /// The byte order the file stores its values in, as its header declares it: what a
    /// run of bytes no instruction claims is read as words in. `Architecture` does not
    /// say it, since MIPS, PowerPC and ARM each come in both. Little for an object made
    /// by [`Object::new`].
    pub endianness: Endianness,
    /// Whether the file is a relocatable object (an ELF `.o`, a COFF `.obj`, a Mach-O `.o`)
    /// rather than a linked image. Its code sections all start where the file says, usually
    /// 0, until [`CodeSection::bias`] places them apart. `false` for an object made by
    /// [`Object::new`].
    pub relocatable: bool,
    pub symbols: HashMap<SymbolIndex, Arc<SymbolData>>,
    /// The same symbols **sorted by name**, byte order, and one name's by index, the file's
    /// order. The Symbols list draws them in this order and a saved place is found in it by
    /// [`Object::symbols_named`]. [`Object::new`] sorts them.
    pub symbols_sorted: Vec<Arc<SymbolData>>,
    /// The functions the file calls and does not define, in its symbol table's order and
    /// then its dynamic one's. They have no code here, so they are not among `symbols`: not
    /// a row in the Symbols list, and a relocation against one names nothing
    /// ([`Operand::Placeholder`](crate::Operand::Placeholder)).
    pub imports: Vec<Import>,
    pub sections: Vec<Arc<Section>>,
    /// The bytes this object was parsed from. See [`ObjectData`].
    pub data: ObjectData,
    /// What went wrong while the object was read, in the order it was found. Empty for a
    /// file that read cleanly. See [`LoadMessage`].
    pub messages: Vec<LoadMessage>,

    /// This object's debug info, built on the first query — except for a PE whose matching
    /// `.pdb` was opened at parse time for the symbols it names, whose backend is seeded
    /// here so it is not opened twice. See [`Object::line_info`].
    pub(crate) debug_info: DebugInfoCache,

    /// The code sections' symbols by the address they are **placed** at, built from
    /// `symbols` by [`Object::new`], so it cannot disagree with them. A symbol left out of
    /// `symbols` has no estimate and no label, and no call is named after it. See
    /// `PlacedSymbols`.
    pub(crate) placed: PlacedSymbols,

    /// See [`Object::is_archive`].
    archive: bool,
    /// See [`Object::placeholder`].
    placeholder: bool,
}

/// Something that went wrong while an object was read, one variant per problem with the
/// data it names. The object is still shown; this says what in it cannot be trusted. How bad
/// it is comes from the variant ([`LoadMessage::severity`]), and so does what the reader is
/// told (its [`Display`](fmt::Display)).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoadMessage {
    /// The code sections could not all be placed apart, because `section` states `address`,
    /// the highest any code section states, near the top of the address space.
    CodeSectionsOverlap { section: String, address: u64 },
    /// `count` functions or entry points were left out because the descriptor naming their
    /// code could not be read.
    UnreadableDescriptors { count: usize },
    /// `count` functions of a relocatable ELF object were left out because their address
    /// could not be worked out: their section does not exist, or its address plus their
    /// offset runs past the end of the address space.
    FunctionsWithoutAddress { count: usize },
    /// `count` relocations in a code section were left out because their section's address
    /// plus their offset runs past the end of the address space. The fields they fill show
    /// the bytes the file holds, with no name.
    RelocationsWithoutAddress { count: usize },
    /// `count` sections are called `<section N>` by their index, because their names could
    /// not be read. They are kept, code and all.
    UnreadableSectionNames { count: usize },
    /// `count` code sections were left out because their bytes would not read or
    /// decompress. Their functions are not shown.
    UnreadableCodeSections { count: usize },
    /// `count` functions the file calls and does not define were left out of its imports
    /// because their names would not read.
    UnreadableImportNames { count: usize },
    /// `count` entries of a linked image's unwind table (`.eh_frame`, `.pdata`) would not
    /// read and were skipped, and, where `cut_short`, the table would not read to its end.
    /// The functions they state may be missing or of estimated length.
    UnreadableUnwindEntries { count: usize, cut_short: bool },
    /// `count` of a linked image's exports would not read and were skipped, and, where
    /// `cut_short`, the export table would not read to its end.
    UnreadableExports { count: usize, cut_short: bool },
    /// A Mach-O image's entry point was left out: its `LC_MAIN` states file offset `offset`,
    /// which no segment's file bytes hold, or whose segment's address plus the offset into
    /// it runs past the end of the address space.
    EntryPointWithoutAddress { offset: u64 },
    /// A Mach-O image has no entry point shown because `count` load commands that state one
    /// (`LC_MAIN`, `LC_UNIXTHREAD`) would not read and were skipped, or, where `cut_short`,
    /// the load commands would not read to their end.
    UnreadableEntryCommands { count: usize, cut_short: bool },
    /// A linked image's entry point was left out because it is at `address`, which no code
    /// section holds.
    EntryPointOutsideCode { address: u64 },
    /// An archive's members stopped at the `member`th (from 1), whose header would not
    /// read, or whose bytes run past the end of the file. Said on the last object shown
    /// before it, or on the archive when none was.
    ArchiveCutShort { member: usize },
    /// A thin archive's `members` are other files, named in it and not held in it, which
    /// this reader does not open.
    ThinArchive { members: usize },
    /// `count` of an archive's members are of a kind this reader knows and does not read,
    /// as `format` says: most often LLVM bitcode, which link-time optimization writes in
    /// place of an object.
    UnsupportedMembers { format: Unsupported, count: usize },
    /// `count` of an archive's members are not object files this reader can read: an
    /// archive inside the archive, or bytes of no known kind.
    UnreadableMembers { count: usize },
    /// An archive that holds no object file at all.
    EmptyArchive,
    /// An archive none of whose members is shown, for the reasons the messages after this
    /// one give: they are thin, not objects this reader can read, or cut short.
    ArchiveShowsNothing,
    /// The file is not an object file or an archive: its first bytes are no kind `object`
    /// knows.
    NotAnObject,
    /// The file starts as an object file or an archive of a kind `object` knows, named by
    /// `format` where this reader has a name for it, and `error` says why it would not
    /// parse: what `object` said, or, where that says too little, this reader's own words.
    Malformed {
        format: Option<Promised>,
        error: String,
    },
    /// The file is of a kind this reader knows and does not read, as `format` says.
    Unsupported { format: Unsupported },
    /// The file could not be read at all, for the reason `error` gives: it is missing, it
    /// is not a regular file, it may not be read, or it changed while it was read.
    CouldNotRead { error: String },
    /// `count` parts of the debug info (a DWARF unit, a PDB module) would not read in whole,
    /// and were passed over or read only up to the fault. Never in
    /// [`Object::messages`]: most are found after the parse, so
    /// [`Object::messages_so_far`] adds this from [`Object::debug_info_skipped`].
    DebugInfoSkipped { count: usize },
}

/// The kind of file a file's first bytes say it is, named when it would not parse
/// ([`LoadMessage::Malformed`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Promised {
    Elf,
    /// A linked Windows image.
    Pe,
    /// A Windows object file.
    Coff,
    MachO,
    Xcoff,
    Archive,
    /// A universal Mach-O, which is not read whole either; named only when it is cut off.
    FatMachO,
    /// A dyld shared cache, likewise.
    DyldCache,
}

/// A kind of file this reader recognizes and does not read ([`LoadMessage::Unsupported`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unsupported {
    /// A universal Mach-O: one binary per architecture in one file.
    FatMachO,
    /// Apple's dyld shared cache, every system library linked into one file.
    DyldCache,
    /// A Windows import library's short import entry, on its own rather than in an archive.
    CoffImport,
    /// LLVM bitcode, bare or in its wrapper: what link-time optimization writes in place of
    /// an object.
    Bitcode,
    /// A WebAssembly module. `object` can read one, but its `wasm` feature is off here.
    Wasm,
    /// An MS-DOS executable with no PE header after its DOS one.
    MsDos,
    /// MSVC's intermediate code, which `cl.exe /GL` writes in place of an object: an
    /// anonymous object header with its class ID.
    ClGl,
    /// Any other anonymous object header (winnt.h's `ANON_OBJECT_HEADER`) that is not a
    /// bigobj COFF file's.
    AnonObject,
    /// A COFF object for a machine `object` does not read, named by its file header's
    /// machine field: one [`coff_machine`] has a name for.
    CoffMachine { machine: u16 },
    /// A GNU ld script, which a distribution installs in place of a shared library's `.so`.
    LinkerScript,
    /// rustc's metadata for a crate: bare, as `--emit=metadata` writes it to an `.rmeta`, or
    /// wrapped in an object with no code, as rustc puts it in an rlib. Only ever a file on its
    /// own: an archive member of it is passed over without a word.
    RustMetadata,
    /// A PDB: a Windows image's debug info, read beside the image rather than on its own.
    Pdb,
    /// An EFI Terse Executable (`VZ`), a PE image with most of its headers stripped, as
    /// firmware's early phases run.
    TerseExecutable,
    /// An archive inside an archive, which is not opened. Only ever an archive member: an
    /// archive on its own is read.
    NestedArchive,
    /// A Java class file, which shares its magic with a universal Mach-O.
    JavaClass,
    /// A PDB in the old 2.00 format, which an image names by a CodeView record this viewer
    /// does not follow.
    OldPdb,
    /// A compressed file, such as a kernel module installed as `.ko.xz`, which is not
    /// decompressed here.
    Compressed { with: Compression },
    /// An Apple text-based stub, the `.tbd` an SDK holds in place of a system library.
    TextStub,
    /// Go's own object file, as the Go compiler writes it into a package's archive.
    GoObject,
}

/// What a compressed file ([`Unsupported::Compressed`]) was compressed with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Compression {
    Gzip,
    Bzip2,
    Xz,
    Zstd,
    Lz4,
    /// The legacy `.lzma` format, which xz reads as `lzma_alone`.
    Lzma,
    Lzip,
}

impl fmt::Display for Compression {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Compression::Gzip => "gzip",
            Compression::Bzip2 => "bzip2",
            Compression::Xz => "xz",
            Compression::Zstd => "zstd",
            Compression::Lz4 => "lz4",
            Compression::Lzma => "lzma",
            Compression::Lzip => "lzip",
        })
    }
}

/// What a COFF object's machine field names, for each machine `object` does not read a
/// COFF object for. `object` names every one of them but 0x0160, big-endian MIPS, and
/// LoongArch's two (`notes/upstream/object.md`).
pub(crate) fn coff_machine(machine: u16) -> Option<&'static str> {
    use object::pe::*;

    Some(match Machine(machine) {
        Machine(0x0160)
        | IMAGE_FILE_MACHINE_R3000
        | IMAGE_FILE_MACHINE_R4000
        | IMAGE_FILE_MACHINE_R10000
        | IMAGE_FILE_MACHINE_WCEMIPSV2
        | IMAGE_FILE_MACHINE_MIPS16
        | IMAGE_FILE_MACHINE_MIPSFPU
        | IMAGE_FILE_MACHINE_MIPSFPU16 => "MIPS",
        IMAGE_FILE_MACHINE_ALPHA => "Alpha",
        IMAGE_FILE_MACHINE_ALPHA64 => "64-bit Alpha",
        IMAGE_FILE_MACHINE_SH3
        | IMAGE_FILE_MACHINE_SH3DSP
        | IMAGE_FILE_MACHINE_SH3E
        | IMAGE_FILE_MACHINE_SH4
        | IMAGE_FILE_MACHINE_SH5 => "SuperH",
        IMAGE_FILE_MACHINE_ARM | IMAGE_FILE_MACHINE_THUMB => "Windows CE's 32-bit ARM",
        IMAGE_FILE_MACHINE_AM33 => "AM33",
        IMAGE_FILE_MACHINE_IA64 => "Itanium",
        IMAGE_FILE_MACHINE_TRICORE => "TriCore",
        IMAGE_FILE_MACHINE_CEF => "CEF",
        IMAGE_FILE_MACHINE_EBC => "EFI byte code",
        IMAGE_FILE_MACHINE_CHPE_X86 => "CHPE x86",
        IMAGE_FILE_MACHINE_RISCV32 => "32-bit RISC-V",
        IMAGE_FILE_MACHINE_RISCV64 => "64-bit RISC-V",
        IMAGE_FILE_MACHINE_RISCV128 => "128-bit RISC-V",
        Machine(0x6232) => "32-bit LoongArch",
        Machine(0x6264) => "64-bit LoongArch",
        IMAGE_FILE_MACHINE_M32R => "M32R",
        IMAGE_FILE_MACHINE_ARM64X => "ARM64X",
        IMAGE_FILE_MACHINE_CEE => "CEE",
        _ => return None,
    })
}

/// How bad a [`LoadMessage`] is. Ordered, so the worst of several is their `max`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Severity {
    /// Something odd that leaves what is shown correct.
    Warning,
    /// Something that makes part of what is shown wrong.
    Error,
    /// The file did not load at all: nothing of it is shown but its name.
    Fatal,
}

impl LoadMessage {
    /// How bad this is, which only the variant decides.
    pub fn severity(&self) -> Severity {
        match self {
            LoadMessage::CodeSectionsOverlap { .. } => Severity::Error,
            LoadMessage::UnreadableDescriptors { .. } => Severity::Warning,
            // What is shown is right; some functions are missing.
            LoadMessage::FunctionsWithoutAddress { .. } => Severity::Warning,
            // What is shown is the file's own bytes; some names are missing.
            LoadMessage::RelocationsWithoutAddress { .. } => Severity::Warning,
            // Only the name is wrong, and it says so.
            LoadMessage::UnreadableSectionNames { .. } => Severity::Warning,
            // What is shown is right; the code in those sections is missing.
            LoadMessage::UnreadableCodeSections { .. } => Severity::Warning,
            // What is shown is right; some imports are missing.
            LoadMessage::UnreadableImportNames { .. } => Severity::Warning,
            // What is shown is right; some functions are missing or of estimated length.
            LoadMessage::UnreadableUnwindEntries { .. } => Severity::Warning,
            // What is shown is right; some names are missing.
            LoadMessage::UnreadableExports { .. } => Severity::Warning,
            // What is shown is right; the entry point is missing.
            LoadMessage::EntryPointWithoutAddress { .. } => Severity::Warning,
            LoadMessage::UnreadableEntryCommands { .. } => Severity::Warning,
            LoadMessage::EntryPointOutsideCode { .. } => Severity::Warning,
            // What is shown is right; only some of it is missing.
            LoadMessage::ArchiveCutShort { .. } => Severity::Warning,
            // Nothing shown is wrong; the members are simply not shown.
            LoadMessage::ThinArchive { .. } => Severity::Warning,
            LoadMessage::UnsupportedMembers { .. } => Severity::Warning,
            LoadMessage::UnreadableMembers { .. } => Severity::Warning,
            // The six below stand for a whole file that shows nothing.
            LoadMessage::EmptyArchive => Severity::Fatal,
            LoadMessage::ArchiveShowsNothing => Severity::Fatal,
            LoadMessage::NotAnObject => Severity::Fatal,
            LoadMessage::Malformed { .. } => Severity::Fatal,
            LoadMessage::Unsupported { .. } => Severity::Fatal,
            LoadMessage::CouldNotRead { .. } => Severity::Fatal,
            // Some code has no source lines; the lines shown are right.
            LoadMessage::DebugInfoSkipped { .. } => Severity::Warning,
        }
    }
}

/// A COFF machine field as the words say it: its name, and the number it is.
struct MachineName(u16);

impl fmt::Display for MachineName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match coff_machine(self.0) {
            Some(name) => write!(f, "{name} (machine {:#06x})", self.0),
            None => write!(f, "machine {:#06x}", self.0),
        }
    }
}

/// What the reader is told: one or two plain sentences.
impl fmt::Display for LoadMessage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LoadMessage::CodeSectionsOverlap { section, address } => write!(
                f,
                "The code sections could not be placed apart: section `{section}` states the \
                 address {address:#x}, near the top of the address space, so addresses in this \
                 object overlap."
            ),
            LoadMessage::UnreadableDescriptors { count } => write!(
                f,
                "Functions left out because their descriptors could not be read: {count}."
            ),
            LoadMessage::FunctionsWithoutAddress { count } => write!(
                f,
                "Functions left out because their address could not be worked out: {count}."
            ),
            LoadMessage::RelocationsWithoutAddress { count } => write!(
                f,
                "Relocations left out because their address could not be worked out: {count}."
            ),
            LoadMessage::UnreadableSectionNames { count } => write!(
                f,
                "Sections named by their index because their names could not be read: {count}."
            ),
            LoadMessage::UnreadableCodeSections { count } => write!(
                f,
                "Code sections left out because their bytes could not be read: {count}."
            ),
            LoadMessage::UnreadableImportNames { count } => write!(
                f,
                "Imports left out because their names could not be read: {count}."
            ),
            LoadMessage::UnreadableUnwindEntries { count, cut_short } => {
                let rest = "The unwind table would not read to its end.";
                match (*count, *cut_short) {
                    (0, _) => write!(f, "{rest}"),
                    (count, false) => write!(f, "Unwind entries that would not read: {count}."),
                    (count, true) => {
                        write!(f, "Unwind entries that would not read: {count}. {rest}")
                    }
                }
            }
            LoadMessage::UnreadableExports { count, cut_short } => {
                let rest = "The export table would not read to its end.";
                match (*count, *cut_short) {
                    (0, _) => write!(f, "{rest}"),
                    (count, false) => write!(f, "Exports that would not read: {count}."),
                    (count, true) => write!(f, "Exports that would not read: {count}. {rest}"),
                }
            }
            LoadMessage::EntryPointWithoutAddress { offset } => write!(
                f,
                "The entry point was left out because the address of its file offset \
                 {offset:#x} could not be worked out."
            ),
            LoadMessage::UnreadableEntryCommands { count, cut_short } => {
                let rest = "The load commands would not read to their end, so the entry point \
                            may be missing.";
                match (*count, *cut_short) {
                    (0, _) => write!(f, "{rest}"),
                    (count, false) => {
                        write!(
                            f,
                            "Load commands stating the entry point that would not read: {count}."
                        )
                    }
                    (count, true) => write!(
                        f,
                        "Load commands stating the entry point that would not read: {count}. {rest}"
                    ),
                }
            }
            LoadMessage::EntryPointOutsideCode { address } => write!(
                f,
                "The entry point was left out because its address {address:#x} is in no code \
                 section."
            ),
            LoadMessage::ArchiveCutShort { member } => write!(
                f,
                "The archive's member {member} would not read, so it and every member after it \
                 are not shown."
            ),
            LoadMessage::ThinArchive { members } => write!(
                f,
                "The archive is thin: its members are in other files, which are not opened. \
                 Members not shown: {members}."
            ),
            LoadMessage::UnsupportedMembers { format, count } => {
                let what = match format {
                    Unsupported::FatMachO => "universal (fat) Mach-O binaries",
                    Unsupported::DyldCache => "dyld shared caches",
                    Unsupported::CoffImport => "Windows import entries",
                    Unsupported::Bitcode => "LLVM bitcode, as link-time optimization writes it",
                    Unsupported::Wasm => "WebAssembly modules",
                    Unsupported::MsDos => "MS-DOS executables with no PE header",
                    Unsupported::ClGl => "MSVC's intermediate code, as cl.exe writes it under /GL",
                    Unsupported::AnonObject => "Windows anonymous objects",
                    Unsupported::LinkerScript => "linker scripts",
                    Unsupported::RustMetadata => "rustc's metadata",
                    Unsupported::Pdb => "PDBs",
                    Unsupported::TerseExecutable => "EFI Terse Executables",
                    Unsupported::NestedArchive => "archives",
                    Unsupported::JavaClass => "Java class files",
                    Unsupported::OldPdb => "PDBs in the old 2.00 format",
                    Unsupported::TextStub => "text-based stubs",
                    Unsupported::GoObject => "Go object files",
                    Unsupported::CoffMachine { machine } => {
                        return write!(
                            f,
                            "Archive members left out because they are COFF objects for {}: \
                             {count}.",
                            MachineName(*machine)
                        );
                    }
                    Unsupported::Compressed { with } => {
                        return write!(
                            f,
                            "Archive members left out because they are compressed with {with}: \
                             {count}."
                        );
                    }
                };
                write!(
                    f,
                    "Archive members left out because they are {what}: {count}."
                )
            }
            LoadMessage::UnreadableMembers { count } => write!(
                f,
                "Archive members left out because they are not object files this reader can \
                 read: {count}."
            ),
            LoadMessage::EmptyArchive => write!(f, "The archive holds no object files."),
            LoadMessage::ArchiveShowsNothing => {
                write!(f, "Nothing in this archive could be shown.")
            }
            LoadMessage::NotAnObject => {
                write!(f, "This file is not an object file or an archive.")
            }
            LoadMessage::Malformed { format, error } => {
                let kind = match format {
                    Some(Promised::Elf) => "an ELF file",
                    Some(Promised::Pe) => "a PE file",
                    Some(Promised::Coff) => "a COFF object file",
                    Some(Promised::MachO) => "a Mach-O file",
                    Some(Promised::Xcoff) => "an XCOFF file",
                    Some(Promised::Archive) => "an archive",
                    Some(Promised::FatMachO) => "a universal (fat) Mach-O binary",
                    Some(Promised::DyldCache) => "a dyld shared cache",
                    None => return write!(f, "This file would not parse: {error}."),
                };
                write!(
                    f,
                    "This looks like {kind}, but it would not parse: {error}."
                )
            }
            LoadMessage::Unsupported { format } => match format {
                Unsupported::FatMachO => write!(
                    f,
                    "This is a universal (fat) Mach-O binary, which this viewer does not read. \
                     `lipo -thin` extracts one architecture from it."
                ),
                Unsupported::DyldCache => write!(
                    f,
                    "This is a dyld shared cache, which this viewer does not read."
                ),
                Unsupported::CoffImport => write!(
                    f,
                    "This is a Windows import entry, which only names a function a DLL exports \
                     and holds no code."
                ),
                Unsupported::Bitcode => write!(
                    f,
                    "This is LLVM bitcode, as link-time optimization writes it, which this \
                     viewer does not read."
                ),
                Unsupported::Wasm => write!(
                    f,
                    "This is a WebAssembly module, which this viewer does not read."
                ),
                Unsupported::MsDos => write!(
                    f,
                    "This is an MS-DOS executable with no PE header, which this viewer does not \
                     read."
                ),
                Unsupported::ClGl => write!(
                    f,
                    "This is MSVC's intermediate code, as cl.exe writes it under /GL, which \
                     this viewer does not read."
                ),
                Unsupported::AnonObject => write!(
                    f,
                    "This is a Windows anonymous object, which this viewer does not read."
                ),
                Unsupported::CoffMachine { machine } => write!(
                    f,
                    "This is a COFF object for {}, which this viewer does not read.",
                    MachineName(*machine)
                ),
                Unsupported::LinkerScript => write!(
                    f,
                    "This is a linker script, which names the libraries to link rather than \
                     holding code."
                ),
                Unsupported::RustMetadata => write!(
                    f,
                    "This is rustc's metadata for a crate, which describes it to the compiler \
                     and holds no code."
                ),
                Unsupported::Pdb => write!(
                    f,
                    "This is a PDB, a Windows image's debug info, which this viewer reads \
                     beside the image it belongs to. Open the image instead."
                ),
                Unsupported::TerseExecutable => write!(
                    f,
                    "This is an EFI Terse Executable, which this viewer does not read."
                ),
                Unsupported::NestedArchive => write!(
                    f,
                    "This is an archive inside an archive, which this viewer does not read."
                ),
                Unsupported::JavaClass => write!(
                    f,
                    "This is a Java class file, which holds JVM bytecode this viewer does not \
                     read."
                ),
                Unsupported::OldPdb => write!(
                    f,
                    "This is a PDB in the old 2.00 format, a Windows image's debug info, which \
                     this viewer does not read."
                ),
                Unsupported::Compressed { with } => write!(
                    f,
                    "This file is compressed with {with}, and this viewer does not decompress \
                     it. Decompress it first."
                ),
                Unsupported::TextStub => write!(
                    f,
                    "This is a text-based stub (.tbd), which names what an Apple library exports \
                     rather than holding code."
                ),
                Unsupported::GoObject => write!(
                    f,
                    "This is a Go object file, in the Go compiler's own format, which this viewer \
                     does not read."
                ),
            },
            LoadMessage::CouldNotRead { error } => {
                write!(f, "This file could not be read: {error}.")
            }
            LoadMessage::DebugInfoSkipped { count } => write!(
                f,
                "Parts of the debug info that would not read, so some code has no source \
                 lines: {count}."
            ),
        }
    }
}

/// [`Object::placed`]: every symbol inside a code section's bytes
/// ([`SymbolData::code_place`]), one [`PlacedSymbol`] each, sorted by address and then by
/// index. Two names at one address are both kept, side by side in the file's order.
///
/// One index serves the whole object because of the placed layout: a linked image's
/// addresses are real, and each code section of a relocatable object has a place of its own.
/// A section that is not code has no place and would collide, so its symbols are left out.
///
/// What a symbol's extent estimate, a listing's labels, a call's name and the source index
/// all read. Built at parse, off the UI thread, because a render asks it too: the history
/// buttons name a saved place with [`Object::symbol_at_placed`], so every ask has to be a
/// binary search and never the sort over every symbol.
pub(crate) struct PlacedSymbols(Vec<PlacedSymbol>);

/// One entry of [`PlacedSymbols`]: a symbol, the index the file names it by, and the address
/// its code is placed at.
pub(crate) struct PlacedSymbol {
    pub(crate) placed: PlacedAddress,
    pub(crate) index: SymbolIndex,
    pub(crate) symbol: Arc<SymbolData>,
    /// Whether the symbol's section is not one of the [`drawn_sections`]: the listing of
    /// all the code does not show its bytes, only another section's or nothing.
    pub(crate) hidden: bool,
}

impl PlacedSymbol {
    /// Whether the symbol is in `section`. Two code sections can overlap where they are
    /// placed, so an entry inside a section's placed range is not always its own.
    pub(crate) fn is_in(&self, section: &Section) -> bool {
        self.symbol
            .section
            .as_ref()
            .is_some_and(|own| std::ptr::eq(Arc::as_ptr(own), section))
    }
}

/// The code sections a listing of all the code draws, each with its placed range, in placed
/// order: every section whose bytes have a place, less any whose range overlaps the one
/// drawn before it. Of two starting at one address, the lower index comes first.
///
/// [`CodeListing`](crate::CodeListing) draws these, and [`Object::symbol_at_placed`] and
/// [`Object::placed_in_code`] skip the symbols of every other section, so a name never
/// disagrees with the code shown at its address.
pub(crate) fn drawn_sections(
    sections: &[Arc<Section>],
) -> Vec<(Arc<Section>, Range<PlacedAddress>)> {
    let mut placed: Vec<_> = sections
        .iter()
        .filter_map(|section| Some((section.clone(), section.placed_range()?)))
        .collect();
    placed.sort_by_key(|(section, range)| (range.start, section.index.0));

    let mut drawn: Vec<(Arc<Section>, Range<PlacedAddress>)> = Vec::with_capacity(placed.len());
    for next in placed {
        if drawn
            .last()
            .is_none_or(|(_, last)| last.end <= next.1.start)
        {
            drawn.push(next);
        }
    }
    drawn
}

impl Object {
    /// An object holding `symbols`, which may come in any order, and no imports. This is
    /// where [`symbols_sorted`](Self::symbols_sorted) and [`placed`](Self::placed) are
    /// sorted, and it starts `debug_info` empty, to be built on its first use.
    pub fn new(
        path: PathBuf,
        name: String,
        format: BinaryFormat,
        architecture: Architecture,
        symbols: HashMap<SymbolIndex, Arc<SymbolData>>,
        sections: Vec<Arc<Section>>,
        data: ObjectData,
    ) -> Object {
        Object::preloaded(
            path,
            name,
            format,
            architecture,
            symbols,
            Vec::new(),
            sections,
            data,
            None,
        )
    }

    /// [`new`](Self::new) with `imports`, and with `debug_info` started on `preloaded`, the
    /// backend the parse already built; [`None`] means nothing is loaded yet, and the first
    /// line question loads it.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn preloaded(
        path: PathBuf,
        name: String,
        format: BinaryFormat,
        architecture: Architecture,
        symbols: HashMap<SymbolIndex, Arc<SymbolData>>,
        imports: Vec<Import>,
        sections: Vec<Arc<Section>>,
        data: ObjectData,
        preloaded: Option<DebugInfo>,
    ) -> Object {
        let mut sorted: Vec<_> = symbols.iter().collect();
        // The map's order is the hash seed's; the file's is the symbol index.
        sorted.sort_unstable_by(|(a_index, a), (b_index, b)| {
            a.name.cmp(&b.name).then(a_index.0.cmp(&b_index.0))
        });
        let symbols_sorted = sorted
            .into_iter()
            .map(|(_, symbol)| symbol.clone())
            .collect();
        let drawn = drawn_sections(&sections);
        let mut placed: Vec<_> = symbols
            .iter()
            .filter_map(|(&index, symbol)| {
                let mut entry = PlacedSymbol {
                    placed: symbol.code_place()?,
                    index,
                    symbol: symbol.clone(),
                    hidden: false,
                };
                entry.hidden = !covering(&drawn, |(_, range)| range.clone(), entry.placed)
                    .is_some_and(|at| entry.is_in(&drawn[at].0));
                Some(entry)
            })
            .collect();
        // The map's order is the hash seed's; the file's is the symbol index.
        placed.sort_unstable_by_key(|entry| (entry.placed, entry.index.0));
        Object {
            path,
            name,
            format: Some(format),
            architecture,
            endianness: Endianness::Little,
            relocatable: false,
            symbols,
            symbols_sorted,
            imports,
            sections,
            data,
            messages: Vec::new(),
            debug_info: DebugInfoCache::new(preloaded),
            placed: PlacedSymbols(placed),
            archive: false,
            placeholder: false,
        }
    }

    /// A file with nothing in it to show, standing in the list for what `messages` say of
    /// it: no format, no sections and no symbols. `archive` is whether the file is one.
    pub(crate) fn unread(
        path: PathBuf,
        name: String,
        data: ObjectData,
        archive: bool,
        messages: Vec<LoadMessage>,
    ) -> Object {
        Object {
            path,
            name,
            format: None,
            architecture: Architecture::Unknown,
            endianness: Endianness::Little,
            relocatable: false,
            symbols: HashMap::new(),
            symbols_sorted: Vec::new(),
            imports: Vec::new(),
            sections: Vec::new(),
            data,
            messages,
            debug_info: DebugInfoCache::new(None),
            placed: PlacedSymbols(Vec::new()),
            archive,
            placeholder: false,
        }
    }

    /// A file whose parse has not landed yet, holding its place in a list until its objects
    /// replace it: no format, no sections, no symbols and nothing to say.
    pub fn placeholder(path: PathBuf) -> Object {
        let name = crate::open::name_of(&path);
        Object {
            placeholder: true,
            ..Object::unread(path, name, ObjectData::from(&[][..]), false, Vec::new())
        }
    }

    /// Whether this is a [`placeholder`](Self::placeholder).
    pub fn is_placeholder(&self) -> bool {
        self.placeholder
    }

    /// Whether this stands for a whole archive, none of whose members is shown: an object
    /// with no [`format`](Self::format) made for an archive. A member of one answers
    /// `false`, being an object in its own right.
    pub fn is_archive(&self) -> bool {
        self.archive
    }

    /// The symbols named exactly `name`, in the file's index order; empty where none is.
    ///
    /// Two binary searches over [`symbols_sorted`](Self::symbols_sorted). It depends on
    /// that field's order, so it lives beside the sort that makes it rather than in the
    /// app that asks: a saved place finds its symbol this way.
    pub fn symbols_named(&self, name: &str) -> &[Arc<SymbolData>] {
        let all = &self.symbols_sorted;
        let start = all.partition_point(|data| data.name.as_str() < name);
        let end = all.partition_point(|data| data.name.as_str() <= name);
        &all[start..end.max(start)]
    }

    /// [`messages`](Self::messages), then what the debug info could not read so far
    /// ([`LoadMessage::DebugInfoSkipped`]), if anything. That grows as the debug info is
    /// read, so this is asked again after each question. It takes one brief lock and reads
    /// nothing.
    pub fn messages_so_far(&self) -> impl Iterator<Item = LoadMessage> + '_ {
        let count = self.debug_info_skipped();
        let skipped = (count > 0).then_some(LoadMessage::DebugInfoSkipped { count });
        self.messages.iter().cloned().chain(skipped)
    }

    /// The worst of [`messages_so_far`](Self::messages_so_far), or [`None`] where there
    /// are none.
    pub fn worst(&self) -> Option<Severity> {
        self.messages_so_far()
            .map(|message| message.severity())
            .max()
    }

    /// [`placed`](Self::placed).
    pub(crate) fn placed_symbols(&self) -> &[PlacedSymbol] {
        &self.placed.0
    }

    /// The entries of [`placed`](Self::placed) whose address is inside `range`.
    pub(crate) fn placed_in(&self, range: Range<PlacedAddress>) -> &[PlacedSymbol] {
        let all = self.placed_symbols();
        let start = all.partition_point(|entry| entry.placed < range.start);
        let end = all.partition_point(|entry| entry.placed < range.end);
        &all[start..end.max(start)]
    }

    /// The text symbol that **starts** at `placed`, in the one address space every section
    /// of this object shares ([`Section::bias`]); [`None`] where no symbol does. Two names
    /// for one address answer the first by name — the order `symbols_sorted` holds — so the
    /// answer is the same however the map behind them was iterated.
    ///
    /// Where two code sections overlap, a symbol of the one the listing of all the code
    /// leaves out ([`drawn_sections`]) is skipped: the name has to be of the code that
    /// listing shows there.
    ///
    /// **Named for the space it answers in**, as `Code::symbol_at_local` is for its own:
    /// the address alone is only a key with the bias in it, and in a relocatable object
    /// every code section starts at 0. A caller holding an address in a section's own terms
    /// adds the section's bias first, as `Code::symbol_at_local` does. In a relocatable
    /// object, one that knows which section the address is in checks the answer is in it
    /// too: the bias makes two sections two places, but a number past one section's end is
    /// still just a number.
    pub fn symbol_at_placed(&self, placed: PlacedAddress) -> Option<&Arc<SymbolData>> {
        first_by_name(self.placed_at(placed).iter().filter(|entry| !entry.hidden))
    }

    /// Where the listing of all the code draws `symbol`: its placed start, or [`None`]
    /// where that listing does not show it -- a symbol in no code section's bytes, or in a
    /// section left out for overlapping another ([`drawn_sections`]). A binary search.
    pub fn placed_in_code(&self, symbol: &SymbolData) -> Option<PlacedAddress> {
        let placed = symbol.code_place()?;
        self.placed_at(placed)
            .iter()
            .any(|entry| std::ptr::eq(Arc::as_ptr(&entry.symbol), symbol) && !entry.hidden)
            .then_some(placed)
    }

    /// The entries of [`placed`](Self::placed) at exactly `placed`.
    pub(crate) fn placed_at(&self, placed: PlacedAddress) -> &[PlacedSymbol] {
        let all = self.placed_symbols();
        let start = all.partition_point(|entry| entry.placed < placed);
        let end = all.partition_point(|entry| entry.placed <= placed);
        &all[start..end.max(start)]
    }
}

/// The symbol of `entries` first by name: the one of two names for an address that
/// [`Object::symbol_at_placed`] answers.
pub(crate) fn first_by_name<'a>(
    entries: impl Iterator<Item = &'a PlacedSymbol>,
) -> Option<&'a Arc<SymbolData>> {
    entries
        .map(|entry| &entry.symbol)
        .min_by(|a, b| a.name.cmp(&b.name))
}

/// A digest of a whole file's bytes: what tells "the same binary" from "one rebuilt
/// underneath the session that named it" (`src/project.rs`). Nothing in this crate reads
/// one; it is computed here because this is where the bytes already are.
///
/// The **content**, not the size and modification time, which are wrong in both directions
/// for the question. xxHash64 because its output is a specified property of the bytes —
/// `std`'s `DefaultHasher` reserves the right to change algorithm between releases, which
/// would declare every saved binary rebuilt after a toolchain upgrade.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct FileDigest(u64);

impl FileDigest {
    pub fn of(bytes: &[u8]) -> FileDigest {
        // Seed 0, xxHash64's own default: part of the algorithm's identity here, so it is
        // written down rather than chosen per run.
        let mut hasher = twox_hash::XxHash64::with_seed(0);
        hasher.write(bytes);
        FileDigest(hasher.finish())
    }
}

/// Sixteen lowercase hex digits, which is the form the session writes.
impl fmt::Display for FileDigest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:016x}", self.0)
    }
}

impl fmt::Debug for FileDigest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "FileDigest({self})")
    }
}

/// The bytes an [`Object`] was parsed from, held for as long as the object lives: parsing
/// keeps decompressed bytes only for the code sections, and whatever reads another one --
/// the line info, the unwind tables -- reads it out of this.
///
/// The bytes are **shared, not copied** — every `Object` out of one file holds a clone of
/// the same `Arc<[u8]>` and differs only in `range` — so an archive costs its bytes once,
/// and one live member keeps the whole archive alive.
#[derive(Clone)]
pub struct ObjectData {
    file: Arc<[u8]>,
    range: Range<usize>,
    /// The digest of the **whole file**, not of `range`: the unit a session names is the
    /// file. [`ObjectData::member`] copies this, so an archive costs one hash and not one
    /// per member.
    digest: FileDigest,
}

impl ObjectData {
    /// The whole file: a plain object file, or the archive file itself. **This is where a
    /// file is hashed**, once, for every object that will come out of it.
    pub fn whole_file(file: Arc<[u8]>) -> Self {
        let range = 0..file.len();
        let digest = FileDigest::of(&file);
        Self {
            file,
            range,
            digest,
        }
    }

    /// One archive member of `file`, as the `(offset, size)` its header declares. [`None`]
    /// when that range does not lie inside the file — the same bounds check
    /// `ArchiveMember::data` does.
    pub fn member(file: &ObjectData, offset: u64, size: u64) -> Option<Self> {
        let start: usize = offset.try_into().ok()?;
        let end = start.checked_add(size.try_into().ok()?)?;
        file.file.get(start..end)?;
        Some(Self {
            file: file.file.clone(),
            range: start..end,
            digest: file.digest,
        })
    }

    /// The object file's own bytes.
    pub fn bytes(&self) -> &[u8] {
        // The range was bounds-checked when it was built.
        &self.file[self.range.clone()]
    }

    /// The digest of the file this object was parsed out of; every object from one file
    /// answers the same thing.
    pub fn digest(&self) -> FileDigest {
        self.digest
    }
}

impl std::fmt::Debug for ObjectData {
    /// Never the bytes themselves: an object file is megabytes of them.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ObjectData")
            .field("range", &self.range)
            .field("file_len", &self.file.len())
            .field("digest", &self.digest)
            .finish()
    }
}

/// Copies the bytes into an allocation of their own, for a caller that only has a slice;
/// [`open_files`](crate::open_files) shares one allocation per file instead.
impl From<&[u8]> for ObjectData {
    fn from(data: &[u8]) -> Self {
        Self::whole_file(Arc::from(data))
    }
}

impl From<Vec<u8>> for ObjectData {
    fn from(data: Vec<u8>) -> Self {
        Self::whole_file(Arc::from(data))
    }
}

#[derive(Debug)]
pub struct Section {
    /// The section's index in the file it was parsed from, which is what identifies it to a
    /// later pass that re-reads that file — an address on its own is not a key in a
    /// relocatable object where every section starts at 0.
    pub index: SectionIndex,
    pub name: String,
    pub address: SectionAddress,

    /// What the parse read of this section, and the one thing that says whether it holds
    /// code: [`Some`] for a section the file marks as code (`SectionKind::Text`) and whose
    /// bytes decompressed, [`None`] for every other. Read through [`code`](Self::code).
    ///
    /// Only a code section's bytes are kept: a debug section is read out of the file when a
    /// line question wants it, so a copy here would be a second one held for the object's
    /// life.
    code: Option<CodeSection>,
}

/// What a section holding code has and no other section does. Reached through
/// [`Section::code`], which is [`Some`] exactly for those.
#[derive(Debug)]
pub struct CodeSection {
    /// The section's bytes, decompressed.
    pub data: Vec<u8>,

    /// The section's relocations by the address the bytes each patches sit at, which is
    /// what a disassembly has to ask by. Not always what the file states: see
    /// [`parse_object`](crate::parse_object). Every one at an address is kept, in the
    /// file's order. Ordered, because the disassembler, the only reader, asks for every
    /// one in an instruction's bytes.
    pub relocations: BTreeMap<SectionAddress, Vec<Relocation>>,

    /// The address ranges the file's own unwind table states for the functions in this
    /// section — an x86-64 PE's `.pdata`, an ELF's `.eh_frame`, out of
    /// [`unwind::entries`](crate::unwind::entries) — each starting in the section's bytes,
    /// sorted by start, each start once, ends clamped to the bytes. Empty for a file with no
    /// table read. What [`SymbolData::extent`] answers from first.
    pub unwind: Vec<Range<SectionAddress>>,

    /// Where the object's layout puts this section: what is added to an address in it to
    /// place it in the one address space every section of the object shares.
    /// [`Bias::NONE`] for every section of a linked image, whose addresses are real; in a
    /// relocatable object, where every code section starts at 0, an address of its own for
    /// each. See [`section_biases`](crate::sections::section_biases).
    pub bias: Bias,
}

impl Section {
    /// A section holding code: its bytes, decompressed, the address they start at, the
    /// relocations in them by address, and its [`bias`](CodeSection::bias). No unwind
    /// ranges.
    pub fn text(
        index: SectionIndex,
        name: String,
        data: Vec<u8>,
        address: SectionAddress,
        relocations: BTreeMap<SectionAddress, Vec<Relocation>>,
        bias: Bias,
    ) -> Section {
        Section {
            index,
            name,
            address,
            code: Some(CodeSection {
                data,
                relocations,
                unwind: Vec::new(),
                bias,
            }),
        }
    }

    /// A section holding no code: no bytes, no relocations, no unwind ranges and no bias.
    pub fn other(index: SectionIndex, name: String, address: SectionAddress) -> Section {
        Section {
            index,
            name,
            address,
            code: None,
        }
    }

    /// What this section holds as code, or [`None`] where it holds none.
    pub fn code(&self) -> Option<&CodeSection> {
        self.code.as_ref()
    }

    /// This section's [`bias`](CodeSection::bias), and [`Bias::NONE`] for a section holding
    /// no code, which has no place in the layout.
    pub fn bias(&self) -> Bias {
        self.code.as_ref().map_or(Bias::NONE, |code| code.bias)
    }

    /// `address`, one of this section's own, in the one address space every section of the
    /// object shares: this section's [`bias`](Self::bias) added. A section holding no code
    /// has no place in the layout, so it answers the same number in the other space. That is the space a
    /// listing of all the object's code draws in and the space
    /// [`Object::symbol_at_placed`] answers in, so anything naming a row places an address
    /// through here.
    ///
    /// Wrapping, and why, is [`SectionAddress::placed`].
    pub fn place(&self, address: SectionAddress) -> PlacedAddress {
        address.placed(self.bias())
    }

    /// [`place`](Self::place) with the overflow said, for a caller that must answer nothing
    /// rather than answer about a different address ([`SectionAddress::placed_checked`]).
    pub(crate) fn place_checked(&self, address: SectionAddress) -> Option<PlacedAddress> {
        address.placed_checked(self.bias())
    }

    /// [`place`](Self::place) saturating, for the ends of a query: an absurd range then asks
    /// about less than it meant to instead of about something else
    /// ([`SectionAddress::placed_saturating`]).
    pub(crate) fn place_saturating(&self, address: SectionAddress) -> PlacedAddress {
        address.placed_saturating(self.bias())
    }

    /// A placed address back in this section's own terms: [`place`](Self::place) undone,
    /// and wrapping for the same reason.
    pub fn local(&self, placed: PlacedAddress) -> SectionAddress {
        placed.local(self.bias())
    }

    /// How many bytes of code this section holds: 0 for one holding none.
    pub(crate) fn len(&self) -> u64 {
        let length = self.code.as_ref().map_or(0, |code| code.data.len());
        // A `usize` is no wider than a `u64` anywhere this builds, so the fallback never
        // answers; it is here so the conversion is not an unwrap.
        length.try_into().unwrap_or(u64::MAX)
    }

    /// Where this section's bytes stop, in the section's own addresses. [`None`] where they
    /// would run past the end of the address space.
    ///
    /// **Checked, and nothing in the crate answers it another way.** A section that does
    /// not fit in the address space names bytes at addresses that do not exist, so it has
    /// no extent, no listing and no place, rather than one of each cut short at
    /// [`u64::MAX`]. The range a symbol is decoded over, the one a listing partitions, the
    /// one an unwind entry is clamped to and the one a declared address is looked up in are
    /// this range, so they cannot say different things.
    pub(crate) fn end(&self) -> Option<SectionAddress> {
        self.address.checked_add(self.len())
    }

    /// The addresses this section's bytes take up, in the section's own terms. [`None`] for
    /// a section with no bytes — one holding no code among them — and for one that does not
    /// fit in the address space ([`end`](Self::end)).
    pub fn bytes_range(&self) -> Option<Range<SectionAddress>> {
        let end = self.end()?;
        (self.address < end).then_some(self.address..end)
    }

    /// This section with `unwind` as its code's [`unwind`](CodeSection::unwind) ranges, made
    /// to hold what that field says: a range not starting in the bytes is dropped, the rest
    /// have their ends clamped to the bytes, and they are sorted by start with each start
    /// kept once. A section holding no code takes none.
    pub(crate) fn with_unwind(mut self, mut unwind: Vec<Range<SectionAddress>>) -> Section {
        let bytes = self.bytes_range();
        let Some(code) = self.code.as_mut() else {
            return self;
        };
        match bytes {
            Some(bytes) => {
                unwind.retain(|range| bytes.contains(&range.start));
                for range in &mut unwind {
                    range.end = range.end.min(bytes.end);
                }
            }
            None => unwind.clear(),
        }
        // By start, and each start once: a table stating one function twice is one
        // function, and the search over them assumes it.
        unwind.sort_unstable_by_key(|range| range.start);
        unwind.dedup_by_key(|range| range.start);
        code.unwind = unwind;
        self
    }

    /// The bytes at `range`, which is in this section's own addresses and not placed ones.
    /// [`None`] where the range is not wholly inside the bytes that were kept — a section
    /// holding no code, a range starting before its address, or one running off its end —
    /// and for a range whose end is before its start.
    ///
    /// Every step is checked, these numbers having come out of a file, and this is the one
    /// place a caller slicing a symbol's code or a gap goes through.
    pub fn bytes_in(&self, range: Range<SectionAddress>) -> Option<&[u8]> {
        let length: usize = range.start.bytes_to(range.end)?.try_into().ok()?;
        let offset: usize = self.address.bytes_to(range.start)?.try_into().ok()?;
        let end = offset.checked_add(length)?;
        self.code.as_ref()?.data.get(offset..end)
    }

    /// The same range placed: [`bytes_range`](Self::bytes_range) put through
    /// [`place`](Self::place). [`None`] wherever that answers [`None`] — a section holding
    /// no code has no place either — and where the layout would put the bytes past the end
    /// of the address space, which `section_biases` never does.
    pub(crate) fn placed_range(&self) -> Option<Range<PlacedAddress>> {
        let bytes = self.bytes_range()?;
        Some(self.place_checked(bytes.start)?..self.place_checked(bytes.end)?)
    }
}

/// A function the file calls and does not define: an undefined text symbol. See
/// [`Object::imports`].
#[derive(Debug)]
pub struct Import {
    /// The file's own spelling, not demangled.
    pub name: String,
    /// The address the file states for it, where it states one: a non-PIE executable's ELF
    /// import is at its PLT slot. [`None`] where the file states 0.
    pub address: Option<SectionAddress>,
}

#[derive(Debug)]
pub struct SymbolData {
    pub name: String,
    pub demangled: Option<String>,
    /// Which name the app made up, where `name` is one of those and not the file's own.
    pub made_up: Option<MadeUp>,
    pub address: SectionAddress,
    pub section: Option<Arc<Section>>,
    /// The size the file states for the symbol, or [`None`] where it states none: an
    /// export, the entry point, a debug file's public, and a symbol table entry whose size
    /// field is 0. Only an ELF's is a function's length ([`Symbol::extent`]).
    pub size: Option<u64>,

    /// What [`extent`](Self::extent) answered, once it has been asked; empty until then.
    pub(crate) extent: ExtentCache,
}

impl SymbolData {
    /// A symbol as the file states it. Its [`extent`](Self::extent) is worked out on the
    /// first ask.
    pub fn new(
        name: String,
        demangled: Option<String>,
        address: SectionAddress,
        section: Option<Arc<Section>>,
        size: Option<u64>,
    ) -> SymbolData {
        SymbolData::parsed(name, demangled, None, address, section, size)
    }

    /// A symbol the parse would have named itself, spelled as it spells `made_up`.
    pub fn new_made_up(
        made_up: MadeUp,
        address: SectionAddress,
        section: Option<Arc<Section>>,
        size: Option<u64>,
    ) -> SymbolData {
        let name = made_up.to_string();
        SymbolData::parsed(name, None, Some(made_up), address, section, size)
    }

    /// [`new`](Self::new) with `made_up` saying which name the parse made up, if it did.
    pub(crate) fn parsed(
        name: String,
        demangled: Option<String>,
        made_up: Option<MadeUp>,
        address: SectionAddress,
        section: Option<Arc<Section>>,
        size: Option<u64>,
    ) -> SymbolData {
        SymbolData {
            name,
            demangled,
            made_up,
            address,
            section,
            size,
            extent: ExtentCache::default(),
        }
    }

    /// What to call this symbol on screen. The disassembler substitutes this for a relocated
    /// operand, so anything rendering a relocation target has to use the same rule.
    pub fn display(&self) -> &str {
        self.demangled.as_deref().unwrap_or(&self.name)
    }

    /// `address`, one of this symbol's own, placed by the section it is in
    /// ([`Section::place`]). A symbol in no section is in no listing either, so nothing
    /// placed it ([`SectionAddress::unplaced`]).
    pub fn placed(&self, address: SectionAddress) -> PlacedAddress {
        self.section
            .as_ref()
            .map_or(address.unplaced(), |section| section.place(address))
    }

    /// Where this symbol starts, in the placed space.
    pub fn placed_start(&self) -> PlacedAddress {
        self.placed(self.address)
    }

    /// Where this symbol is in [`Object::placed`]: its placed address, where its section is
    /// code and the address is inside the section's bytes. [`None`] for every other symbol,
    /// which no listing labels and no estimate is made for.
    pub(crate) fn code_place(&self) -> Option<PlacedAddress> {
        self.place_in(&self.section.as_ref()?.placed_range()?)
    }

    /// This symbol's placed address, where `range` — the placed bytes of the section it is
    /// in — covers it. [`code_place`](Self::code_place) is this with the ask for the range,
    /// so a caller holding one already comes here and asks for it once.
    pub(crate) fn place_in(&self, range: &Range<PlacedAddress>) -> Option<PlacedAddress> {
        let placed = self.placed_start();
        range.contains(&placed).then_some(placed)
    }

    /// The addresses this symbol's [`extent`](Self::extent) covers: what
    /// [`data_in`](Self::data_in) slices, [`assembly`](Self::assembly) decodes and
    /// [`line_info`](Self::line_info) asks about. `extent` has already checked the sum;
    /// it is checked again rather than assumed, as every number here came out of a file.
    pub(crate) fn range(&self, object: &Object) -> Option<Range<SectionAddress>> {
        let bytes = self.extent(object)?.bytes;
        Some(self.address..self.address.checked_add(bytes)?)
    }

    /// This symbol's bytes over its [`range`](Self::range), or [`None`] when that runs
    /// off the end of what was decompressed.
    pub fn data_in(&self, object: &Object) -> Option<&[u8]> {
        self.section.as_ref()?.bytes_in(self.range(object)?)
    }

    /// This symbol's disassembly, or [`None`] when there are no bytes to decode. An
    /// architecture no backend claims comes back as an [`Assembly`] whose
    /// [`undecodable`](Assembly::undecodable) names it.
    ///
    /// The answer **carries the range it was decoded over** and the [`Extent`](crate::Extent)
    /// behind it ([`Assembly::range`]), which is the one place that decision is made for a
    /// symbol the reader is looking at: the line info is asked over that range and the bar
    /// prints its length, neither of them asking [`extent`](Self::extent) again.
    pub fn assembly(&self, object: &Object) -> Option<Arc<Assembly>> {
        let extent = self.extent(object)?;
        let range = self.range(object)?;
        let bytes = self.section.as_ref()?.bytes_in(range.clone())?;
        let code = Code::new(bytes, self.address, self.section.as_deref(), object);
        Some(Arc::new(Assembly::decode(
            object.architecture,
            &code,
            range,
            extent,
        )))
    }
}

/// A symbol together with the object it came from. Identity is `Arc` pointer identity, never
/// name or index, so duplicate symbol names across objects stay distinct.
#[derive(Clone)]
pub struct Symbol {
    pub object: Arc<Object>,
    pub data: Arc<SymbolData>,
}

impl PartialEq for Symbol {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.object, &other.object) && Arc::ptr_eq(&self.data, &other.data)
    }
}

impl Eq for Symbol {}

/// The two pointers the equality above compares, so a map keyed by a symbol takes that
/// identity from here rather than spelling it out again.
impl Hash for Symbol {
    fn hash<H: Hasher>(&self, state: &mut H) {
        Arc::as_ptr(&self.object).hash(state);
        Arc::as_ptr(&self.data).hash(state);
    }
}

/// Which of `items`, a list sorted by range start, holds `address`: the index of the last
/// one starting at or before it, where that one's range contains it.
///
/// [`None`] in three cases, which is every way an address can miss. `items` is empty, or
/// `address` is below the first start, so there is no candidate at all; or the candidate
/// ends at or before `address`, the gap after a range.
///
/// **Only that one candidate is looked at.** Where ranges nest, an address past an inner
/// range but still inside the outer one answers [`None`] rather than the outer one — this
/// finds the last range starting at or before the address, and nothing else.
pub(crate) fn covering<T, A: Ord>(
    items: &[T],
    range: impl Fn(&T) -> Range<A>,
    address: A,
) -> Option<usize> {
    let index = items
        .partition_point(|item| range(item).start <= address)
        .checked_sub(1)?;
    range(&items[index]).contains(&address).then_some(index)
}

/// Ranges that may overlap, each carrying a value, looked up by address: the value of the
/// first range, in the order they were given, that holds it. What `find` over the list
/// answers, for the cost of a [`covering`] search rather than a walk per address.
///
/// Built by cutting the ranges at every start and end: each piece belongs to the first
/// range over it, or to none, so the pieces are disjoint and one search finds the answer.
pub(crate) struct FirstCovering<A, T> {
    /// Disjoint and sorted by start. A piece nothing covers is left out.
    pieces: Vec<(Range<A>, T)>,
}

impl<A: Ord + Copy, T: Copy> FirstCovering<A, T> {
    /// From ranges in the order that decides which one an address in two of them is
    /// taken to be in. An empty or backwards range holds nothing and is dropped.
    pub(crate) fn new(ranges: impl IntoIterator<Item = (Range<A>, T)>) -> Self {
        let ranges: Vec<(Range<A>, T)> = ranges
            .into_iter()
            .filter(|(range, _)| range.start < range.end)
            .collect();
        // Where a range starts or ends, with its place in the order and which of the two.
        let mut edges: Vec<(A, usize, bool)> = ranges
            .iter()
            .enumerate()
            .flat_map(|(order, (range, _))| [(range.start, order, true), (range.end, order, false)])
            .collect();
        edges.sort_unstable_by_key(|&(at, ..)| at);

        // The ranges open between one edge and the next, by their place in the order.
        let mut open = BTreeSet::new();
        let mut pieces: Vec<(Range<A>, usize)> = Vec::new();
        let mut edges = edges.into_iter().peekable();
        while let Some(&(at, ..)) = edges.peek() {
            while let Some((_, order, starts)) = edges.next_if(|&(edge, ..)| edge == at) {
                if starts {
                    open.insert(order);
                } else {
                    open.remove(&order);
                }
            }
            let (Some(&first), Some(&(next, ..))) = (open.first(), edges.peek()) else {
                continue;
            };
            match pieces.last_mut() {
                Some((piece, owner)) if piece.end == at && *owner == first => piece.end = next,
                _ => pieces.push((at..next, first)),
            }
        }

        FirstCovering {
            pieces: pieces
                .into_iter()
                .map(|(piece, owner)| (piece, ranges[owner].1))
                .collect(),
        }
    }

    /// The value of the first range holding `address`, or [`None`] where none does.
    pub(crate) fn get(&self, address: A) -> Option<T> {
        let index = covering(&self.pieces, |(piece, _)| piece.clone(), address)?;
        Some(self.pieces[index].1)
    }

    /// Whether no range holds anything.
    pub(crate) fn is_empty(&self) -> bool {
        self.pieces.is_empty()
    }
}

#[cfg(test)]
mod tests;

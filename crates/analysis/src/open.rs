//! The crate's entry point: each file read, tried as an archive and as an object, and every
//! object in it handed over as it is parsed.

use crate::model::coff_machine;
use crate::parse::parse_unshared;
use crate::{
    open_regular, Compression, Links, LoadMessage, Object, ObjectData, Promised, Regular,
    Unsupported,
};
use object::read::archive::ArchiveFile;
use object::FileKind;
use std::{
    ops::ControlFlow,
    path::{Path, PathBuf},
    sync::Arc,
};

/// What [`open_files_streaming`] has to say as it goes. There is deliberately no *start*:
/// the caller supplied the paths and they are walked in order.
pub enum Progress {
    /// One object, parsed and ready to be read.
    Parsed(Arc<Object>),
    /// Nothing more will come out of this path. Emitted for **every** path the walk reaches,
    /// including one that could not be read and one that yielded nothing at all.
    Finished(PathBuf),
}

/// Parse each path as an archive (contributing one [`Object`] per member) or, if it is not
/// one, as a plain object file, handing each object to `emit` **as it is parsed** rather than
/// collecting them.
///
/// A file that yields no object is handed over all the same, as an object with no format
/// saying why: it could not be read ([`LoadMessage::CouldNotRead`]), it is not an object
/// ([`LoadMessage::NotAnObject`]), it is a kind this reader does not read
/// ([`LoadMessage::Unsupported`]), or it would not parse ([`LoadMessage::Malformed`]). An
/// archive's members come one member late, so the last can say what was left out: members
/// that stopped early ([`LoadMessage::ArchiveCutShort`]), members of a kind this reader does
/// not read, such as LLVM bitcode ([`LoadMessage::UnsupportedMembers`]), or otherwise not
/// objects ([`LoadMessage::UnreadableMembers`]), or a thin archive's, which are not read
/// ([`LoadMessage::ThinArchive`]). An archive with no member shown is handed over alone, with
/// no format, to say so ([`LoadMessage::ArchiveShowsNothing`]) and why, or only that it holds
/// none ([`LoadMessage::EmptyArchive`]).
///
/// A callback rather than a channel or an iterator: a channel would make this crate pick a
/// backpressure policy belonging to whoever draws the result, and an iterator would mean
/// self-borrowing the file's bytes across a yield.
///
/// **`emit` answers whether to go on**, which is how work nobody is waiting for stops. The
/// one thing a single answer cannot express is "skip the rest of *this* file but go on to the
/// next", so a multi-file request in which one file is closed goes on parsing that file's
/// remaining members and drops them at the caller.
pub fn open_files_streaming(
    paths: Vec<PathBuf>,
    mut emit: impl FnMut(Progress) -> ControlFlow<()>,
) {
    for path in paths {
        // The path may be one a project file lists, so it is read as any path a file chose
        // is: a fifo or a device is refused, and so is a file that reads more than it states.
        let flow = match open_regular(&path, Links::Follow).and_then(Regular::read_stated) {
            Ok(bytes) => open_one_file(&path, Arc::from(bytes), &mut emit),
            Err(error) => unread_file(&path, &error, &mut emit),
        };
        if flow.is_break() {
            return;
        }
    }
}

/// A path that could not be read, handed over as an object saying why. There are no bytes,
/// so it holds none.
fn unread_file(
    path: &Path,
    error: &std::io::Error,
    emit: &mut impl FnMut(Progress) -> ControlFlow<()>,
) -> ControlFlow<()> {
    let message = LoadMessage::CouldNotRead {
        error: error.to_string(),
    };
    let data = ObjectData::from(&[][..]);
    let object = Object::unread(
        path.to_path_buf(),
        name_of(path),
        data,
        false,
        vec![message],
    );
    emit(Progress::Parsed(Arc::new(object)))?;
    emit(Progress::Finished(path.to_path_buf()))
}

/// The same for files already in memory, each with the path it is to be called by:
/// everything [`open_files_streaming`] does except the reading.
///
/// Bytes in hand cannot fail to be read, so this is that walk without its unreadable-path
/// case ([`LoadMessage::CouldNotRead`]). It is for a caller holding a file it has no reason
/// to write out first — the tests, which build their archives with the `object` writer and
/// would otherwise need a directory to put them in.
pub fn open_data_streaming(
    files: Vec<(PathBuf, Arc<[u8]>)>,
    mut emit: impl FnMut(Progress) -> ControlFlow<()>,
) {
    for (path, bytes) in files {
        if open_one_file(&path, bytes, &mut emit).is_break() {
            return;
        }
    }
}

/// One file's worth of either, split out so `?` on the caller's answer reads as what it
/// is: abandon this file.
fn open_one_file(
    path: &Path,
    bytes: Arc<[u8]>,
    emit: &mut impl FnMut(Progress) -> ControlFlow<()>,
) -> ControlFlow<()> {
    // One allocation per file, shared by every object parsed out of it and held for as long
    // as they live. Built here rather than where it is used at the bottom, because it is what
    // the members are cut from: the hash it takes is then one pass over the file however many
    // objects come out of it.
    let file = ObjectData::whole_file(bytes);
    let name = name_of(path);

    // An archive's magic is no object's, so a file that parses as one is not tried as both.
    let archive = match ArchiveFile::parse(file.bytes()) {
        Ok(archive) => archive,
        Err(error) => {
            // A file that is not an archive is an object file or nothing, and one that is
            // an archive whose tables would not read is only that.
            let kind = FileKind::parse(file.bytes());
            let archive = matches!(kind, Ok(FileKind::Archive));
            let object = match archive {
                true => Err(error),
                false => parse_unshared(file.clone(), name.clone(), path.to_path_buf()),
            };
            let object = object.unwrap_or_else(|error| {
                let message = match kind {
                    Ok(kind) => match unsupported(kind, file.bytes()) {
                        Some(format) => LoadMessage::Unsupported { format },
                        None => LoadMessage::Malformed {
                            format: promised(kind),
                            error: error.to_string(),
                        },
                    },
                    Err(_) => told_by_magic(file.bytes()).unwrap_or(LoadMessage::NotAnObject),
                };
                Object::unread(path.to_path_buf(), name, file, archive, vec![message])
            });
            emit(Progress::Parsed(Arc::new(object)))?;
            return emit(Progress::Finished(path.to_path_buf()));
        }
    };

    // Each object is handed over one member late, so the last one can still be told what
    // went wrong with the members.
    let mut held: Option<Object> = None;
    // How many members a thin archive names, none of which is read here.
    let mut thin = 0usize;
    // How many members were of each kind this reader does not read, in the order the kinds
    // were first met, and how many were otherwise not objects it can read.
    let mut unsupported: Vec<(Unsupported, usize)> = Vec::new();
    let mut unreadable = 0usize;
    let mut cut_short = None;
    for (at, member) in archive.members().enumerate() {
        let stopped = LoadMessage::ArchiveCutShort {
            member: at.saturating_add(1),
        };
        // `object` ends the walk at the first member it cannot read, though the size
        // in its header may be good and the members after it fine.
        let Ok(member) = member else {
            cut_short = Some(stopped);
            break;
        };
        // Its bytes are in a file of its own, and `file_range` says offset 0 and that
        // file's size: a range into this archive's own first bytes.
        if member.is_thin() {
            thin = thin.saturating_add(1);
            continue;
        }
        // The same bytes `member.data(..)` would return, addressed as a range into the
        // archive so the member stays reachable without re-scanning the archive. A range
        // past the end is a file cut short inside this member, which `object`'s walk ends
        // at as well.
        let (offset, size) = member.file_range();
        let Some(data) = ObjectData::member(&file, offset, size) else {
            cut_short = Some(stopped);
            break;
        };
        let expected = not_code(member.name(), data.bytes());
        let kind = unsupported_member(data.bytes());
        let name = String::from_utf8_lossy(member.name()).into_owned();
        match parse_unshared(data, name, path.to_path_buf()) {
            Ok(object) => {
                if let Some(previous) = held.replace(object) {
                    emit(Progress::Parsed(Arc::new(previous)))?;
                }
            }
            Err(_) if expected => {}
            Err(_) => match kind {
                None => unreadable = unreadable.saturating_add(1),
                Some(kind) => match unsupported.iter_mut().find(|(met, _)| *met == kind) {
                    Some((_, count)) => *count = count.saturating_add(1),
                    None => unsupported.push((kind, 1)),
                },
            },
        }
    }

    let mut said = Vec::new();
    if thin > 0 {
        said.push(LoadMessage::ThinArchive { members: thin });
    }
    for (format, count) in unsupported {
        said.push(LoadMessage::UnsupportedMembers { format, count });
    }
    if unreadable > 0 {
        said.push(LoadMessage::UnreadableMembers { count: unreadable });
    }
    said.extend(cut_short);
    match held {
        Some(mut last) => {
            last.messages.extend(said);
            emit(Progress::Parsed(Arc::new(last)))?;
        }
        // An archive does not parse as an object, so with no member shown there is no row
        // to say this on but one made for it. Showing nothing is fatal, so a fatal message
        // heads the warnings that say why.
        None => {
            let nothing = if said.is_empty() {
                LoadMessage::EmptyArchive
            } else {
                LoadMessage::ArchiveShowsNothing
            };
            said.insert(0, nothing);
            let object = Object::unread(path.to_path_buf(), name, file, true, said);
            emit(Progress::Parsed(Arc::new(object)))?;
        }
    }
    emit(Progress::Finished(path.to_path_buf()))
}

/// Which kind of file `bytes`, which `object` tells as `kind`, are, if it is one `object`
/// recognizes and will not parse: every kind `object::File::parse` does not take but an
/// archive, which is read apart.
fn unsupported(kind: FileKind, bytes: &[u8]) -> Option<Unsupported> {
    match kind {
        // `object` takes `CAFEBABE` for a universal Mach-O, and a Java class file starts with
        // it too. The next four bytes are the architecture count in one and the version,
        // minor then major, in the other, so they are told apart as LLVM's `identify_magic`
        // does: a count under 43 is a Mach-O's, and a class file's major version is at least
        // 45. `file(1)` draws the line likewise, at under 20 and over 30.
        FileKind::MachOFat32 => match bytes.get(4..8) {
            Some(&[a, b, c, d]) if u32::from_be_bytes([a, b, c, d]) >= 43 => {
                Some(Unsupported::JavaClass)
            }
            _ => Some(Unsupported::FatMachO),
        },
        FileKind::MachOFat64 => Some(Unsupported::FatMachO),
        FileKind::DyldCache => Some(Unsupported::DyldCache),
        FileKind::CoffImport => Some(Unsupported::CoffImport),
        _ => None,
    }
}

/// What `kind` is called when a file of it would not parse, for every kind that reaches the
/// parse; `object` may add kinds this has no name for.
fn promised(kind: FileKind) -> Option<Promised> {
    match kind {
        FileKind::Elf32 | FileKind::Elf64 => Some(Promised::Elf),
        FileKind::Pe32 | FileKind::Pe64 => Some(Promised::Pe),
        FileKind::Coff | FileKind::CoffBig => Some(Promised::Coff),
        FileKind::MachO32 | FileKind::MachO64 => Some(Promised::MachO),
        FileKind::Xcoff32 | FileKind::Xcoff64 => Some(Promised::Xcoff),
        FileKind::Archive => Some(Promised::Archive),
        _ => None,
    }
}

/// Which kind of file `bytes` is, if it is one `object` does not recognize and this reader
/// knows by its magic: LLVM bitcode, bare or in its wrapper, a WebAssembly module, which
/// `object` would read were its `wasm` feature on, rustc's metadata on its own, a PDB in
/// either format, a GNU ld script, an Apple text-based stub, a Go object file, and a
/// compressed file.
fn unknown_to_object(bytes: &[u8]) -> Option<Unsupported> {
    match bytes {
        [b'B', b'C', 0xc0, 0xde, ..] | [0xde, 0xc0, 0x17, 0x0b, ..] => Some(Unsupported::Bitcode),
        [0x00, b'a', b's', b'm', ..] => Some(Unsupported::Wasm),
        // rustc's `METADATA_HEADER`, less the version byte after it.
        [b'r', b'u', b's', b't', 0, 0, 0, ..] => Some(Unsupported::RustMetadata),
        _ if bytes.starts_with(PDB_MAGIC) => Some(Unsupported::Pdb),
        _ if bytes.starts_with(OLD_PDB_MAGIC) => Some(Unsupported::OldPdb),
        _ if linker_script(bytes) => Some(Unsupported::LinkerScript),
        _ if text_stub(bytes) => Some(Unsupported::TextStub),
        _ if bytes.starts_with(b"go object ") => Some(Unsupported::GoObject),
        _ => compressed(bytes).map(|with| Unsupported::Compressed { with }),
    }
}

/// What `bytes` are compressed with, by the magic each format starts with, and for the two
/// with a short magic the byte after it: gzip's method, which is always deflate, and bzip2's
/// block size, a digit from 1 to 9. lzip's version byte is 0 or 1. The legacy `.lzma`
/// format has no magic, so its whole header is checked instead ([`lzma_alone`]).
fn compressed(bytes: &[u8]) -> Option<Compression> {
    match bytes {
        [0x1f, 0x8b, 0x08, ..] => Some(Compression::Gzip),
        [b'B', b'Z', b'h', b'1'..=b'9', ..] => Some(Compression::Bzip2),
        [0xfd, b'7', b'z', b'X', b'Z', 0, ..] => Some(Compression::Xz),
        [0x28, 0xb5, 0x2f, 0xfd, ..] => Some(Compression::Zstd),
        [0x04, 0x22, 0x4d, 0x18, ..] => Some(Compression::Lz4),
        [b'L', b'Z', b'I', b'P', 0 | 1, ..] => Some(Compression::Lzip),
        _ if lzma_alone(bytes) => Some(Compression::Lzma),
        _ => None,
    }
}

/// Whether `bytes` start with a legacy `.lzma` header as xz-utils' `lzma_alone` decoder
/// accepts one when it has to guess the format. The header is 13 bytes: a properties byte,
/// here the default `5d`; a dictionary size, here one whose two low bytes are 0, as they are
/// from 128 KiB up; and the uncompressed size. Like xz, it takes only a dictionary of 2^n or
/// 2^n + 2^(n-1) bytes, and a size that is unknown (all `ff`) or under 256 GiB. The format
/// has no magic, so that is what keeps other bytes from passing for one.
fn lzma_alone(bytes: &[u8]) -> bool {
    let Some(&[0x5d, 0, 0, d2, d3, ref size @ ..]) = bytes.first_chunk::<13>() else {
        return false;
    };
    let dictionary = u32::from_le_bytes([0, 0, d2, d3]);
    // xz's own test, which rounds up to the nearest allowed size.
    let mut d = dictionary.wrapping_sub(1);
    d |= d >> 2;
    d |= d >> 3;
    d |= d >> 4;
    d |= d >> 8;
    d |= d >> 16;
    let size = u64::from_le_bytes(*size);
    d.wrapping_add(1) == dictionary && (size == u64::MAX || size < 1 << 38)
}

/// The first 32 bytes of a PDB, the MSF 7.00 superblock's magic.
const PDB_MAGIC: &[u8] = b"Microsoft C/C++ MSF 7.00\r\n\x1aDS\0\0\0";

/// The first 44 bytes of a PDB in the old 2.00 format. `pdb2` reads one, but `object` finds
/// an image's PDB only by the RSDS CodeView record that names a 7.00 one, and an image with
/// a 2.00 PDB names it by an NB10 record.
const OLD_PDB_MAGIC: &[u8] = b"Microsoft C/C++ program database 2.00\r\n\x1aJG\0\0";

/// Whether `bytes` are a GNU ld script as a distribution installs one in place of a shared
/// library's `.so`: glibc's start with the comment saying so, and gcc's are a bare `INPUT` or
/// `GROUP` command.
fn linker_script(bytes: &[u8]) -> bool {
    let command = |word: &[u8]| {
        bytes
            .strip_prefix(word)
            .is_some_and(|rest| rest.trim_ascii_start().starts_with(b"("))
    };
    bytes.starts_with(b"/* GNU ld script") || command(b"INPUT") || command(b"GROUP")
}

/// Whether `bytes` are an Apple text-based stub, as LLVM's `identify_magic` tells one: YAML
/// with the `!tapi` tag of versions 2 to 4, or version 1's, which has no tag and starts with
/// its `archs`. Version 5 is JSON, told by its first key: its version, or the library it
/// describes, which LLVM's writer puts first, as it sorts the keys.
fn text_stub(bytes: &[u8]) -> bool {
    let json = bytes.strip_prefix(b"{").is_some_and(|rest| {
        let rest = rest.trim_ascii_start();
        rest.starts_with(b"\"tapi_tbd_version\"") || rest.starts_with(b"\"main_library\"")
    });
    bytes.starts_with(b"--- !tapi") || bytes.starts_with(b"---\narchs:") || json
}

/// The class ID of the anonymous object MSVC's `cl.exe /GL` writes, as LLVM's
/// `ClGlObjMagic` has it.
const CL_GL_CLASS_ID: [u8; 16] = [
    0x38, 0xfe, 0xb3, 0x0c, 0xa5, 0xd9, 0xab, 0x4d, 0xac, 0x9b, 0xd6, 0xb6, 0x22, 0x26, 0x53, 0xc2,
];

/// What a file whose kind `object` cannot tell is, where its magic says: a kind this reader
/// does not read, or one it reads that would not parse, and why. `object` reads 16 bytes to
/// tell any kind, and some kinds more.
fn told_by_magic(bytes: &[u8]) -> Option<LoadMessage> {
    if let Some(format) = unknown_to_object(bytes) {
        return Some(LoadMessage::Unsupported { format });
    }
    let malformed = |format, error| {
        Some(LoadMessage::Malformed {
            format: Some(format),
            error,
        })
    };
    let ends = || format!("the file ends after {} bytes", bytes.len());
    let short = bytes.len() < 16;
    match bytes {
        [0x7f, b'E', b'L', b'F', ..] if short => malformed(Promised::Elf, ends()),
        [0x7f, b'E', b'L', b'F', class, ..] => malformed(
            Promised::Elf,
            format!("its class byte is {class}, not 1 (32-bit) or 2 (64-bit)"),
        ),
        [0xfe, 0xed, 0xfa, 0xce | 0xcf, ..] | [0xce | 0xcf, 0xfa, 0xed, 0xfe, ..] if short => {
            malformed(Promised::MachO, ends())
        }
        // A Java class file starts `CAFEBABE` too, so one is told apart by the four bytes
        // after the magic as [`unsupported`] tells it, and not at all without them.
        [0xca, 0xfe, 0xba, 0xbe, _, _, _, _, ..] if short => {
            match unsupported(FileKind::MachOFat32, bytes) {
                Some(Unsupported::JavaClass) => Some(LoadMessage::Unsupported {
                    format: Unsupported::JavaClass,
                }),
                _ => malformed(Promised::FatMachO, ends()),
            }
        }
        [0xca, 0xfe, 0xba, 0xbf, ..] if short => malformed(Promised::FatMachO, ends()),
        [b'd', b'y', b'l', b'd', b'_', b'v', b'1', b' ', ..] if short => {
            malformed(Promised::DyldCache, ends())
        }
        [b'M', b'Z', ..] => ms_dos(bytes),
        [b'V', b'Z', ..] => terse_executable(bytes),
        // An anonymous object header, whose class ID, bytes 12 to 28, says what it is.
        // `object` reads version 0 as an import entry, and version 2 with bigobj's class ID
        // as COFF.
        [0, 0, 0xff, 0xff, ..] if bytes.get(12..28) == Some(&CL_GL_CLASS_ID) => {
            Some(LoadMessage::Unsupported {
                format: Unsupported::ClGl,
            })
        }
        [0, 0, 0xff, 0xff, 2, 0, ..] if bytes.len() < 28 => malformed(Promised::Coff, ends()),
        [0, 0, 0xff, 0xff, 2, 0, ..] => malformed(
            Promised::Coff,
            "its header's class ID is not a bigobj file's".to_owned(),
        ),
        [0, 0, 0xff, 0xff, low, high, ..] if [*low, *high] != [0, 0] => {
            Some(LoadMessage::Unsupported {
                format: Unsupported::AnonObject,
            })
        }
        _ => other_coff_machine(bytes),
    }
}

/// Whether `object` reads a COFF object for `machine`: whether `FileKind::parse` takes a file
/// that starts with it for COFF.
fn object_reads_coff(machine: u16) -> bool {
    let mut header = [0; 16];
    header[..2].copy_from_slice(&machine.to_le_bytes());
    matches!(FileKind::parse(&header[..]), Ok(FileKind::Coff))
}

/// [`told_by_magic`] for an EFI Terse Executable, whose magic is `VZ`. That is two letters,
/// so it is named only when its 40-byte header names a COFF machine and at least one section,
/// and the section table is inside the file.
fn terse_executable(bytes: &[u8]) -> Option<LoadMessage> {
    let &[_, _, m0, m1, sections, ..] = bytes else {
        return None;
    };
    let machine = u16::from_le_bytes([m0, m1]);
    let known = object_reads_coff(machine) || coff_machine(machine).is_some();
    let table_end = usize::from(sections).checked_mul(40)?.checked_add(40)?;
    let fits = known && sections > 0 && table_end <= bytes.len();
    fits.then_some(LoadMessage::Unsupported {
        format: Unsupported::TerseExecutable,
    })
}

/// [`told_by_magic`] for a COFF object whose machine `object` does not read. The machine
/// field is two bytes, too little to go on alone, so it is named only when `object` parses
/// the rest as a COFF object: its section table and its symbol table inside the file.
/// `CoffFile::parse` does not check the machine; `FileKind::parse` is what refuses it.
fn other_coff_machine(bytes: &[u8]) -> Option<LoadMessage> {
    let file = object::read::coff::CoffFile::<&[u8]>::parse(bytes).ok()?;
    let machine = file.coff_header().machine.get(object::LittleEndian).0;
    coff_machine(machine)?;
    Some(LoadMessage::Unsupported {
        format: Unsupported::CoffMachine { machine },
    })
}

/// [`told_by_magic`] for a file that starts `MZ`, which `object` reads as a PE file only when
/// the DOS header points at a PE header with a PE32 or PE32+ magic. Each step is read with
/// `object`'s own headers; one with no PE signature is an MS-DOS program.
fn ms_dos(bytes: &[u8]) -> Option<LoadMessage> {
    use object::pe::{ImageDosHeader, ImageNtHeaders32, IMAGE_NT_SIGNATURE};
    use object::read::pe::{ImageNtHeaders as _, ImageOptionalHeader as _};
    use object::ReadRef as _;

    let malformed = |error: String| {
        Some(LoadMessage::Malformed {
            format: Some(Promised::Pe),
            error,
        })
    };
    let dos = match ImageDosHeader::parse(bytes) {
        Ok(dos) => dos,
        Err(error) => return malformed(error.to_string()),
    };
    let at = u64::from(dos.nt_headers_offset());
    let Ok(nt) = bytes.read_at::<ImageNtHeaders32>(at) else {
        // Too short for a PE header: a PE signature there still says what it was meant to be.
        return match bytes.read_bytes_at(at, 4) {
            Ok(b"PE\0\0") => malformed(format!(
                "the file ends after {} bytes, inside its PE header",
                bytes.len()
            )),
            _ => Some(LoadMessage::Unsupported {
                format: Unsupported::MsDos,
            }),
        };
    };
    if nt.signature() != IMAGE_NT_SIGNATURE {
        return Some(LoadMessage::Unsupported {
            format: Unsupported::MsDos,
        });
    }
    malformed(format!(
        "its optional header's magic is {:#x}, not PE32's 0x10b or PE32+'s 0x20b",
        nt.optional_header().magic()
    ))
}

/// Which kind of file an archive member is, if it is one this reader knows and does not read.
/// That is every kind [`told_by_magic`] and [`unsupported`] name, and an archive: one on its
/// own is read, but one inside another is not opened. An archive is told by its magic, since
/// an empty one is 8 bytes, too short for `object` to tell any kind.
fn unsupported_member(bytes: &[u8]) -> Option<Unsupported> {
    if bytes.starts_with(b"!<arch>\n") || bytes.starts_with(b"!<thin>\n") {
        return Some(Unsupported::NestedArchive);
    }
    match FileKind::parse(bytes) {
        Ok(kind) => unsupported(kind, bytes),
        Err(_) => match told_by_magic(bytes) {
            Some(LoadMessage::Unsupported { format }) => Some(format),
            _ => None,
        },
    }
}

/// What an object out of `path` is called when it is the whole file: its file name, or the
/// whole path where it has none.
pub(crate) fn name_of(path: &Path) -> String {
    match path.file_name() {
        Some(name) => name.to_string_lossy().into_owned(),
        None => path.display().to_string(),
    }
}

/// Whether an archive member that did not parse was never meant to hold code, so leaving it
/// out is nothing to report: rustc's metadata in an rlib, which is an object on the targets
/// rustc knows how to wrap it for and bare bytes elsewhere, the Go compiler's export data in
/// a Go package's archive, which starts as a Go object file does, and an import library's
/// short entries, each only a name the DLL exports. The archive's symbol and name tables are
/// not among the members `object` hands over at all.
fn not_code(name: &[u8], bytes: &[u8]) -> bool {
    name == b"lib.rmeta"
        || name == b"lib.rmeta-link"
        || name == b"__.PKGDEF"
        || matches!(FileKind::parse(bytes), Ok(FileKind::CoffImport))
}

/// [`open_files_streaming`] with the objects collected, for a caller with nowhere to put them
/// one at a time.
pub fn open_files(paths: Vec<PathBuf>) -> Vec<Arc<Object>> {
    let mut objects = Vec::new();
    open_files_streaming(paths, |progress| {
        if let Progress::Parsed(object) = progress {
            objects.push(object);
        }
        ControlFlow::Continue(())
    });
    objects
}

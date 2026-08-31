use iced_x86::Formatter;
use object::{
    read::archive::ArchiveFile, CompressionFormat, Object as _, ObjectSection, ObjectSymbol,
    Relocation, RelocationTarget, SectionIndex, SymbolIndex, SymbolKind,
};
use std::{collections::HashMap, fs, ops::Range, path::PathBuf, sync::Arc};
use symbolic_demangle::{Demangle, DemangleOptions};

pub use object::BinaryFormat;

pub struct Object {
    pub path: PathBuf,
    pub name: String,
    pub format: BinaryFormat,
    pub symbols: HashMap<SymbolIndex, Arc<SymbolData>>,
    pub symbols_sorted: Vec<Arc<SymbolData>>,
    pub sections: Vec<Arc<Section>>,
    /// The bytes this object was parsed from. See [`ObjectData`].
    pub data: ObjectData,
}

/// The bytes an [`Object`] was parsed from, held for as long as the object lives.
///
/// Parsing keeps decompressed bytes only for the sections that hold text symbols, so an
/// `Object` on its own cannot be asked anything about, say, its `.debug_*` sections after
/// the fact. Keeping the input around lets a later, lazy pass (line info) build what it
/// needs without re-reading the file — and it keeps an archive *member* addressable, which
/// re-reading could not do cheaply: a member is a slice of the archive, so finding it
/// again would mean reading and re-scanning the whole archive.
///
/// The bytes are **shared, not copied**: `open_files` reads each file once and every
/// `Object` it yields from that file — one per archive member *plus* one for the file
/// itself — holds a clone of the same `Arc<[u8]>` and differs only in `range`, which is
/// the extent of *its* object file within it (the whole file for a plain object, the
/// member's slice for an archive member). Copying each member's bytes instead would hold
/// an archive's contents roughly twice over, since the archive file is parsed as a plain
/// object as well.
///
/// **Memory cost:** these are exactly the bytes `open_files` already reads; the only
/// change is that they are now retained for the object's lifetime instead of being dropped
/// when parsing returns. A file costs its own size once however many objects come out of
/// it — 3.5 MiB for the sample `librustc_data_structures-*.rlib`, 137 MiB for the sample
/// `LLVM-24-rust-dev.dll` — and `fs::read` yields a `Vec`, so the conversion to `Arc<[u8]>`
/// copies it once and the peak while opening is briefly twice the file's size. The flip
/// side of sharing is that one live archive member keeps the whole archive's bytes alive,
/// which is the right trade for a viewer that lists every member anyway.
///
/// This allocation is exactly what the `[?]` "Prefer memory mapped files and minimal
/// memory footprint" goal in `notes/Goals.md` would replace: mapping the file instead of
/// reading it turns this into an `Arc` of a mapping, at which point the resident cost is
/// the kernel's page cache and the transient copy disappears too.
#[derive(Clone)]
pub struct ObjectData {
    file: Arc<[u8]>,
    range: Range<usize>,
}

impl ObjectData {
    /// The whole file: a plain object file, or the archive file itself.
    pub fn whole_file(file: Arc<[u8]>) -> Self {
        let range = 0..file.len();
        Self { file, range }
    }

    /// One archive member, as the `(offset, size)` its header declares. [`None`] when
    /// that range does not lie inside the file, which is the same bounds check
    /// `ArchiveMember::data` does — such a member is skipped, exactly as before.
    pub fn member(file: &Arc<[u8]>, offset: u64, size: u64) -> Option<Self> {
        let start: usize = offset.try_into().ok()?;
        let end = start.checked_add(size.try_into().ok()?)?;
        file.get(start..end)?;
        Some(Self {
            file: file.clone(),
            range: start..end,
        })
    }

    /// The object file's own bytes.
    pub fn bytes(&self) -> &[u8] {
        // The range was bounds-checked when it was built.
        &self.file[self.range.clone()]
    }
}

impl std::fmt::Debug for ObjectData {
    /// Never the bytes themselves: an object file is megabytes of them.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ObjectData")
            .field("range", &self.range)
            .field("file_len", &self.file.len())
            .finish()
    }
}

/// Copies the bytes into an allocation of their own. Convenient for callers that only
/// have a slice (the tests); [`open_files`] shares one allocation per file instead.
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
    pub name: String,
    pub data: Vec<u8>,
    pub address: u64,

    pub relocations: HashMap<u64, Relocation>,

    // A sorted list of symbol positions
    pub symbols: Vec<u64>,
}

#[derive(Debug)]
pub struct SymbolData {
    pub name: String,
    pub demangled: Option<String>,
    pub address: u64,
    pub section: Option<Arc<Section>>,
    pub size: u64,
}

impl SymbolData {
    /// Object files frequently report a size of 0, so derive the extent from the next
    /// symbol in the section (or the section end).
    pub fn estimate_size(&self) -> Option<u64> {
        let section = self.section.as_ref()?;
        let i = section.symbols.binary_search(&self.address).ok()?;
        if i + 1 == section.symbols.len() {
            section
                .address
                .checked_add(section.data.len().try_into().ok()?)?
                .checked_sub(self.address)
        } else {
            section.symbols[i + 1].checked_sub(self.address)
        }
    }

    pub fn data(&self) -> Option<&[u8]> {
        let section = self.section.as_ref()?;
        let size: usize = self.estimate_size()?.try_into().ok()?;
        let offset: usize = self.address.checked_sub(section.address)?.try_into().ok()?;
        let end = offset.checked_add(size)?;
        section.data.get(offset..end)
    }

    pub fn assembly(&self, object: &Object) -> Option<Arc<Assembly>> {
        let bytes = self.data()?;
        let mut decoder =
            iced_x86::Decoder::with_ip(64, bytes, self.address, iced_x86::DecoderOptions::NONE);

        let mut formatter = iced_x86::IntelFormatter::new();

        formatter.options_mut().set_first_operand_char_index(10);
        formatter
            .options_mut()
            .set_space_after_operand_separator(true);

        let mut instruction = iced_x86::Instruction::default();

        let mut assembly = Assembly {
            instructions: Vec::new(),
        };

        while decoder.can_decode() {
            decoder.decode_out(&mut instruction);

            let start_index = (instruction.ip() - self.address) as usize;

            let mut relocation = None;

            self.section.as_ref().map(|section| {
                for i in 0..instruction.len() {
                    section
                        .relocations
                        .get(&(instruction.ip() + i as u64))
                        .map(|r| {
                            relocation = Some(r.target().clone());
                        });
                }
            });

            let relocation = relocation.and_then(|r| match r {
                RelocationTarget::Symbol(i) => object.symbols.get(&i).cloned(),
                _ => None,
            });

            let mut inst = Instruction {
                address: instruction.ip(),
                bytes: bytes[start_index..start_index + instruction.len()].to_vec(),
                format: Vec::new(),
                relocation,
            };
            formatter.format(&instruction, &mut inst);

            assembly.instructions.push(inst);
        }

        Some(Arc::new(assembly))
    }
}

/// A symbol together with the object it came from. Identity is `Arc` pointer identity,
/// never name or index, so duplicate symbol names across objects stay distinct.
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

/// The kind of a formatted assembly text span, as far as the UI is concerned.
///
/// This is the disassembler-independent stand-in for `iced_x86::FormatterTextKind`,
/// so that nothing outside this crate has to depend on a particular backend.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SpanKind {
    Mnemonic,
    Prefix,
    Register,
    Number,
    /// A branch target: the address operand of a `call`, `jmp` or `jcc`.
    Address,
    Other,
}

impl From<iced_x86::FormatterTextKind> for SpanKind {
    fn from(kind: iced_x86::FormatterTextKind) -> Self {
        match kind {
            iced_x86::FormatterTextKind::Mnemonic => SpanKind::Mnemonic,
            iced_x86::FormatterTextKind::Prefix => SpanKind::Prefix,
            iced_x86::FormatterTextKind::Register => SpanKind::Register,
            iced_x86::FormatterTextKind::Number => SpanKind::Number,
            // A near-branch target comes through `write_number` as one of these two:
            // `FunctionAddress` when the branch is a call, `LabelAddress` for a plain
            // jump or a conditional one. iced-x86 has no `BranchTarget` kind.
            iced_x86::FormatterTextKind::LabelAddress
            | iced_x86::FormatterTextKind::FunctionAddress => SpanKind::Address,
            _ => SpanKind::Other,
        }
    }
}

#[derive(Clone)]
pub struct Instruction {
    pub address: u64,
    pub bytes: Vec<u8>,
    pub format: Vec<(String, SpanKind)>,
    pub relocation: Option<Arc<SymbolData>>,
}

impl iced_x86::FormatterOutput for Instruction {
    fn write(&mut self, text: &str, kind: iced_x86::FormatterTextKind) {
        self.format.push((text.to_owned(), kind.into()));
    }

    fn write_number(
        &mut self,
        _instruction: &iced_x86::Instruction,
        _operand: u32,
        _instruction_operand: Option<u32>,
        text: &str,
        _value: u64,
        _number_kind: iced_x86::NumberKind,
        kind: iced_x86::FormatterTextKind,
    ) {
        // The placeholder value in the encoding is meaningless when a relocation
        // applies; the target symbol name is rendered instead.
        if self.relocation.is_none() {
            self.write(text, kind);
        }
    }
}

pub struct Assembly {
    pub instructions: Vec<Instruction>,
}

/// A hard ceiling on how large a single section's decompressed bytes may be, whatever
/// its header claims. See [`section_data`].
const MAX_SECTION_DATA: u64 = 1 << 30;

/// Read a section's bytes, decompressing it if it says it is compressed, but only after
/// checking that the size it declares is believable.
///
/// `uncompressed_data()` trusts the size in the compression header and reserves that much
/// *before* it looks at a single compressed byte, so a corrupt or hostile header — one
/// flipped `SHF_COMPRESSED` bit in a 608-byte file is enough — turns into a multi-gigabyte
/// allocation and an OOM abort on a small machine. `compressed_data()` hands us the same
/// information without allocating: the declared `uncompressed_size` plus the compressed
/// bytes themselves, which are a slice of the file and so are already bounded by it.
///
/// Two independent bounds have to hold, and a section failing either is dropped exactly
/// like one whose name or data will not read — no panic, no error, it simply has no data:
///
/// * A ratio bound, which is provable rather than a guess. DEFLATE cannot expand data by
///   more than 1032:1, and a zstd frame not by more than 32768:1 (a 128 KiB block stored
///   as a 4-byte RLE block), so a declared size past that is a lie about *these* bytes no
///   matter what they decode to. As the compressed bytes come from the file, this also
///   caps the allocation at a multiple of the file's own length.
/// * An absolute bound, because the ratio bound alone still scales with the input: a
///   100 MB object could honestly claim 100 GB. 1 GiB is far more than anything this
///   viewer can show — it keeps only sections holding text symbols, and it already holds
///   the whole file in memory besides.
///
/// Both are orders of magnitude above real files: compressed debug sections run at
/// roughly 2:1 to 10:1, so nothing legitimate comes anywhere near either limit.
fn section_data<'data, S: ObjectSection<'data>>(section: &S) -> Option<Vec<u8>> {
    let compressed = section.compressed_data().ok()?;

    let max_ratio: u64 = match compressed.format {
        // Not compressed at all: the bytes are already there, nothing to bound.
        CompressionFormat::None => return Some(compressed.data.to_vec()),
        CompressionFormat::Zlib => 1032,
        CompressionFormat::Zstandard => 32768,
        // Any other format is one `decompress()` does not implement; it would fail.
        _ => return None,
    };

    let ratio_bound = (compressed.data.len() as u64).saturating_mul(max_ratio);
    if compressed.uncompressed_size > ratio_bound.min(MAX_SECTION_DATA) {
        return None;
    }

    Some(compressed.decompress().ok()?.into_owned())
}

/// Parse `data` as a single object file. `name` is the display name (an archive member
/// name or the file name) and `path` the file it came from. Anything that fails to
/// parse yields [`None`].
///
/// `data` is kept in the returned [`Object`]; see [`ObjectData`]. A caller with nothing
/// but bytes (the tests) can pass `bytes.into()`, which gives them an allocation of their
/// own; [`open_files`] shares one per file.
pub fn parse_object(data: ObjectData, name: String, path: PathBuf) -> Option<Arc<Object>> {
    let object = object::File::parse(data.bytes())
        .map(|file| {
            let mut sections: HashMap<SectionIndex, Section> = file
                .sections()
                .filter_map(|section| {
                    let name = String::from_utf8_lossy(section.name_bytes().ok()?).into_owned();
                    let data = section_data(&section)?;
                    let relocations = section.relocations().collect();
                    Some((
                        section.index(),
                        Section {
                            name,
                            address: section.address(),
                            data,
                            symbols: Vec::new(),
                            relocations,
                        },
                    ))
                })
                .collect();

            // Insert symbol addresses into sections
            file.symbols().for_each(|symbol| {
                if symbol.kind() != SymbolKind::Text {
                    return;
                }

                symbol
                    .section()
                    .index()
                    .and_then(|index| sections.get_mut(&index))
                    .map(|section| section.symbols.push(symbol.address()));
            });

            let section_map: HashMap<SectionIndex, Arc<Section>> = sections
                .into_iter()
                .map(|(index, mut section)| {
                    section.symbols.sort_unstable();
                    (index, Arc::new(section))
                })
                .collect();

            let sections = section_map.values().cloned().collect();

            let symbols: HashMap<_, _> = file
                .symbols()
                .filter_map(|symbol| {
                    // Filter out non-text symbols
                    (symbol.kind() == SymbolKind::Text).then(|| ())?;

                    let name = String::from_utf8_lossy(symbol.name_bytes().ok()?).into_owned();
                    let demangled =
                        symbolic_common::Name::from(&name).demangle(DemangleOptions::complete());

                    let section = symbol
                        .section()
                        .index()
                        .and_then(|index| section_map.get(&index).cloned());

                    Some((
                        symbol.index(),
                        Arc::new(SymbolData {
                            name,
                            demangled,
                            section,
                            address: symbol.address(),
                            size: symbol.size(),
                        }),
                    ))
                })
                .collect();

            let mut symbols_sorted: Vec<_> = symbols.values().cloned().collect();
            symbols_sorted.sort_unstable_by(|a, b| a.name.cmp(&b.name));

            ParsedObject {
                format: file.format(),
                symbols,
                symbols_sorted,
                sections,
            }
        })
        .ok()?;

    // Nothing above borrows the file any more -- sections own decompressed copies of
    // their bytes and relocations are owned values -- so the input can be moved in.
    Some(Arc::new(Object {
        name,
        path,
        format: object.format,
        symbols: object.symbols,
        symbols_sorted: object.symbols_sorted,
        sections: object.sections,
        data,
    }))
}

/// Everything [`parse_object`] reads out of the file, i.e. an [`Object`] minus the fields
/// that do not come from parsing. It exists only so the borrow of `data` ends before
/// `data` itself is moved into the object.
struct ParsedObject {
    format: BinaryFormat,
    symbols: HashMap<SymbolIndex, Arc<SymbolData>>,
    symbols_sorted: Vec<Arc<SymbolData>>,
    sections: Vec<Arc<Section>>,
}

fn open_object(out: &mut Vec<Arc<Object>>, data: ObjectData, name: String, path: PathBuf) {
    out.extend(parse_object(data, name, path));
}

/// Parse each path as an archive (contributing one [`Object`] per member) *and* as a
/// plain object file. Anything that fails to read or parse is silently skipped.
pub fn open_files(paths: Vec<PathBuf>) -> Vec<Arc<Object>> {
    let mut objects = Vec::new();

    for path in paths {
        let Ok(file) = fs::read(&path) else {
            continue;
        };

        // One allocation per file, shared by every object parsed out of it and held for
        // as long as they live; see `ObjectData`. `fs::read` gives a `Vec`, so this
        // copies the bytes once more before dropping the original.
        let file: Arc<[u8]> = Arc::from(file);

        if let Ok(archive) = ArchiveFile::parse(&*file) {
            for member in archive.members() {
                member
                    .map(|member| {
                        let name = String::from_utf8_lossy(member.name()).into_owned();
                        // The same bytes `member.data(..)` would return, addressed as a
                        // range into the archive so the member stays reachable from the
                        // object without the archive having to be scanned again.
                        let (offset, size) = member.file_range();
                        if let Some(data) = ObjectData::member(&file, offset, size) {
                            open_object(&mut objects, data, name, path.clone());
                        }
                    })
                    .ok();
            }
        }

        open_object(
            &mut objects,
            ObjectData::whole_file(file),
            path.file_name()
                .map(|name| name.to_string_lossy())
                .unwrap_or_default()
                .into_owned(),
            path.clone(),
        );
    }

    objects
}

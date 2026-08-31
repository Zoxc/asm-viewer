use iced_x86::Formatter;
use object::{
    read::archive::ArchiveFile, Object as _, ObjectSection, ObjectSymbol, Relocation,
    RelocationTarget, SectionIndex, SymbolIndex, SymbolKind,
};
use std::{collections::HashMap, fs, path::PathBuf, sync::Arc};
use symbolic_demangle::{Demangle, DemangleOptions};

pub use object::BinaryFormat;

pub struct Object {
    pub path: PathBuf,
    pub name: String,
    pub format: BinaryFormat,
    pub symbols: HashMap<SymbolIndex, Arc<SymbolData>>,
    pub symbols_sorted: Vec<Arc<SymbolData>>,
    pub sections: Vec<Arc<Section>>,
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
    Other,
}

impl From<iced_x86::FormatterTextKind> for SpanKind {
    fn from(kind: iced_x86::FormatterTextKind) -> Self {
        match kind {
            iced_x86::FormatterTextKind::Mnemonic => SpanKind::Mnemonic,
            iced_x86::FormatterTextKind::Prefix => SpanKind::Prefix,
            iced_x86::FormatterTextKind::Register => SpanKind::Register,
            iced_x86::FormatterTextKind::Number => SpanKind::Number,
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

fn open_object(out: &mut Vec<Arc<Object>>, data: &[u8], name: String, path: PathBuf) {
    object::File::parse(data)
        .map(|file| {
            let mut sections: HashMap<SectionIndex, Section> = file
                .sections()
                .filter_map(|section| {
                    let name = String::from_utf8_lossy(section.name_bytes().ok()?).into_owned();
                    let data = section.uncompressed_data().ok()?.into_owned();
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

            out.push(Arc::new(Object {
                name,
                path,
                format: file.format(),
                symbols,
                symbols_sorted,
                sections,
            }));
        })
        .ok();
}

/// Parse each path as an archive (contributing one [`Object`] per member) *and* as a
/// plain object file. Anything that fails to read or parse is silently skipped.
pub fn open_files(paths: Vec<PathBuf>) -> Vec<Arc<Object>> {
    let mut objects = Vec::new();

    for path in paths {
        let Ok(file) = fs::read(&path) else {
            continue;
        };

        if let Ok(archive) = ArchiveFile::parse(file.as_slice()) {
            for member in archive.members() {
                member
                    .map(|member| {
                        let name = String::from_utf8_lossy(member.name()).into_owned();
                        member
                            .data(file.as_slice())
                            .map(|data| {
                                open_object(&mut objects, data, name, path.clone());
                            })
                            .ok();
                    })
                    .ok();
            }
        }

        open_object(
            &mut objects,
            file.as_slice(),
            path.file_name()
                .map(|name| name.to_string_lossy())
                .unwrap_or_default()
                .into_owned(),
            path.clone(),
        );
    }

    objects
}

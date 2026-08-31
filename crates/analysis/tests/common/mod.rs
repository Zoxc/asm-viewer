//! In-memory fixture builders. Nothing here touches the filesystem or commits a blob:
//! every test object is assembled with the `object` crate's writer.

#![allow(dead_code)]

use object::write;
use object::{
    Architecture, BinaryFormat, Endianness, RelocationEncoding, RelocationKind, SymbolFlags,
    SymbolKind, SymbolScope,
};

pub struct TextSymbol<'a> {
    pub name: &'a str,
    pub bytes: &'a [u8],
}

/// A relocation inside the generated `.text`: `(symbol index, offset within that
/// symbol, index of the symbol it targets)`.
pub struct TextRelocation {
    pub in_symbol: usize,
    pub offset: u64,
    pub target: usize,
}

/// Build a minimal x86-64 ELF relocatable object whose `.text` holds `symbols` laid out
/// back to back, each declared with `st_size == 0` — the common case that
/// `SymbolData::estimate_size` exists to work around.
pub fn elf_x86_64(symbols: &[TextSymbol], relocations: &[TextRelocation]) -> Vec<u8> {
    let mut obj = write::Object::new(BinaryFormat::Elf, Architecture::X86_64, Endianness::Little);
    let text = obj.section_id(write::StandardSection::Text);

    let mut offsets = Vec::new();
    let mut ids = Vec::new();

    for symbol in symbols {
        let offset = obj.append_section_data(text, symbol.bytes, 1);
        offsets.push(offset);
        ids.push(obj.add_symbol(write::Symbol {
            name: symbol.name.as_bytes().to_vec(),
            value: offset,
            // Deliberately 0: object files frequently report no size at all.
            size: 0,
            kind: SymbolKind::Text,
            scope: SymbolScope::Linkage,
            weak: false,
            section: write::SymbolSection::Section(text),
            flags: SymbolFlags::None,
        }));
    }

    for relocation in relocations {
        obj.add_relocation(
            text,
            write::Relocation {
                offset: offsets[relocation.in_symbol] + relocation.offset,
                size: 32,
                kind: RelocationKind::Relative,
                encoding: RelocationEncoding::X86Branch,
                symbol: ids[relocation.target],
                addend: -4,
            },
        )
        .expect("adding a relocation to .text");
    }

    obj.write().expect("writing the fixture object")
}

/// The fixture from the plan: `caller` = `call rel32; ret` with a relocation at offset 1
/// pointing at `target` = `ret`.
pub fn caller_and_target() -> Vec<u8> {
    elf_x86_64(
        &[
            TextSymbol {
                name: "caller",
                bytes: &[0xE8, 0x00, 0x00, 0x00, 0x00, 0xC3],
            },
            TextSymbol {
                name: "target",
                bytes: &[0xC3],
            },
        ],
        &[TextRelocation {
            in_symbol: 0,
            offset: 1,
            target: 1,
        }],
    )
}

/// Deterministic pseudo-random bytes (xorshift64*), so a failure is always reproducible.
pub fn garbage(seed: u64, len: usize) -> Vec<u8> {
    let mut state = seed | 1;
    (0..len)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state >> 24) as u8
        })
        .collect()
}

//! Unwind entries: the `RUNTIME_FUNCTION`s an x86-64 PE's exception directory (`.pdata`)
//! states, one per function with unwind info, each with both ends and no name.

mod common;

use common::{pe_image, ExportedSymbol, PeDll, TEXT_ADDRESS};
use object::Object as _;

/// Four functions back to back, each `nop`s then a `ret`, so every offset below is a real
/// instruction boundary and a listing decoded from it terminates.
const TEXT: &[u8] = &[
    0x90, 0x90, 0x90, 0xC3, // 0: three nops and a ret
    0x90, 0xC3, // 4
    0xC3, // 6
    0x90, 0x90, 0xC3, // 7
];

const FIRST: ExportedSymbol = ExportedSymbol {
    name: "first",
    offset: 0,
    size: 0,
    code: true,
};

/// The in-memory writer's `.pdata` reads back through `object` as a linker's does: a
/// section by that name, and an exception data directory of exactly the entries asked for,
/// three RVAs each.
#[test]
fn the_writers_pdata_reads_back_through_object() {
    let image = pe_image(PeDll {
        text: TEXT,
        symbols: &[FIRST],
        entry: None,
        codeview: None,
        unwind: &[(0, 4), (4, 6)],
    });
    let file = object::File::parse(image.as_slice()).expect("a PE image");
    assert!(file.section_by_name(".pdata").is_some());
    let text_rva = (TEXT_ADDRESS - file.relative_address_base()) as u32;

    let object::File::Pe64(pe) = &file else {
        panic!("the writer emits PE32+");
    };
    let directory = pe
        .data_directories()
        .get(object::pe::IMAGE_DIRECTORY_ENTRY_EXCEPTION)
        .expect("an exception directory");
    let data = directory
        .data(pe.data(), &pe.section_table())
        .expect("the directory's bytes");
    let words: Vec<u32> = data
        .chunks_exact(4)
        .map(|word| u32::from_le_bytes(word.try_into().unwrap()))
        .collect();
    assert_eq!(words.len(), 6, "two entries of three words");
    assert_eq!(words[0..2], [text_rva, text_rva + 4]);
    assert_eq!(words[3..5], [text_rva + 4, text_rva + 6]);
}

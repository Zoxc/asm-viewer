//! Unwind entries: the `RUNTIME_FUNCTION`s an x86-64 PE's exception directory (`.pdata`)
//! states, one per function with unwind info, each with both ends and no name.

mod common;

use analysis::{Architecture, Gap, GapKind, Listing};
use common::{
    elf_shared_object, named, names, parse, pe_image, ExportedSymbol, PeDll, SharedObject,
    TEXT_ADDRESS,
};
use object::{Object as _, ObjectSection as _};

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
        fragments: &[],
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

const SECOND: ExportedSymbol = ExportedSymbol {
    name: "second",
    offset: 4,
    size: 0,
    code: true,
};

/// An image with `first` exported and entries on it and on two functions nothing names.
fn image_with(entry: Option<u64>, unwind: &[(u64, u64)]) -> Vec<u8> {
    pe_image(PeDll {
        text: TEXT,
        symbols: &[FIRST],
        entry,
        codeview: None,
        unwind,
        fragments: &[],
    })
}

/// A function only its unwind entry declares is a symbol, named `<function 0x…>` by its
/// address since the entry carries no name, with the entry's length as its declared size,
/// and it disassembles like any other.
#[test]
fn unwind_entries_are_symbols_where_nothing_names_them() {
    let object = parse(&image_with(None, &[(0, 4), (4, 6), (7, 10)]));
    assert_eq!(
        names(&object),
        [
            format!("<function {:#x}>", TEXT_ADDRESS + 4),
            format!("<function {:#x}>", TEXT_ADDRESS + 7),
            "first".to_owned(),
        ]
    );

    let second = named(&object, &format!("<function {:#x}>", TEXT_ADDRESS + 4));
    assert_eq!(second.address, TEXT_ADDRESS + 4);
    assert_eq!(second.size, 2, "the entry's stated length");
    assert_eq!(second.demangled, None, "ours, not the file's");
    assert_eq!(
        second.section.as_ref().map(|s| s.name.as_str()),
        Some(".text")
    );
    let assembly = second.assembly(&object).expect("it decodes");
    assert!(!assembly.instructions.is_empty());
}

/// An entry at an address something else already names — an export, the entry point — adds
/// no second symbol, and an entry that states nothing (empty, inverted, or beginning outside
/// any code section) adds none at all.
#[test]
fn an_entry_at_a_named_address_adds_no_symbol_and_a_malformed_one_nothing() {
    let object = parse(&pe_image(PeDll {
        text: TEXT,
        symbols: &[FIRST, SECOND],
        entry: Some(6),
        codeview: None,
        // Past `.text`'s page is `.rdata`.
        unwind: &[(0, 4), (4, 6), (6, 7), (4, 4), (9, 8), (0x1000, 0x1004)],
        fragments: &[],
    }));
    assert_eq!(names(&object), ["<entry point>", "first", "second"]);
    assert_eq!(object.symbols.len(), 3);
    let section = named(&object, "first").section.clone().unwrap();
    assert_eq!(
        section.symbols,
        [TEXT_ADDRESS, TEXT_ADDRESS + 4, TEXT_ADDRESS + 6]
    );
    assert_eq!(
        section.unwind,
        [
            TEXT_ADDRESS..TEXT_ADDRESS + 4,
            TEXT_ADDRESS + 4..TEXT_ADDRESS + 6,
            TEXT_ADDRESS + 6..TEXT_ADDRESS + 7,
        ],
        "the three that state something"
    );
}

/// ARM64's `.pdata` is another record altogether, so an image for it is not read as
/// x86-64 entries however its exception directory is laid out.
#[test]
fn an_arm64_images_pdata_is_not_read_as_x86_64_entries() {
    let mut image = image_with(None, &[(0, 4), (4, 6), (7, 10)]);
    // `Machine`, the first word of the COFF header at `e_lfanew`: IMAGE_FILE_MACHINE_ARM64.
    image[0x44..0x46].copy_from_slice(&0xAA64u16.to_le_bytes());
    let object = parse(&image);
    assert_eq!(object.architecture, Architecture::Aarch64);
    assert_eq!(names(&object), ["first"]);
}

/// The end an entry states is the function's, padding excluded, where the next symbol's
/// address is not: the extent stops at the stated end, the bytes from there to the next
/// symbol are the listing's gap, and `estimate_size` — the derivation, by name — still says
/// what it always said.
#[test]
fn a_stated_end_beats_the_next_symbols_address() {
    const TEXT: &[u8] = &[
        0x90, 0x90, 0x90, 0x90, 0x90, 0xC3, // 0: five nops and a ret
        0xCC, 0xCC, 0xCC, 0xCC, // 6: the linker's int3 padding
        0x90, 0xC3, // 10
    ];
    let object = parse(&pe_image(PeDll {
        text: TEXT,
        symbols: &[
            FIRST,
            ExportedSymbol {
                name: "second",
                offset: 10,
                size: 0,
                code: true,
            },
        ],
        entry: None,
        codeview: None,
        unwind: &[(0, 6), (10, 12)],
        fragments: &[],
    }));
    let first = named(&object, "first");
    assert_eq!(first.estimate_size(), Some(10));
    assert_eq!(first.debug_extent(&object), None, "no debug info at all");
    assert_eq!(first.extent(&object), Some(6));
    assert_eq!(first.data(), Some(&TEXT[..10]), "the derivation, by name");
    assert_eq!(first.data_in(&object), Some(&TEXT[..6]));
    let assembly = first.assembly(&object).expect("first decodes");
    assert_eq!(assembly.instructions.len(), 6);

    let listing = Listing::new(&object, first.section.clone().unwrap());
    let stretch = listing.decode(&object, 0).expect("first decodes");
    assert_eq!(
        stretch.gap,
        Some(Gap {
            range: TEXT_ADDRESS + 6..TEXT_ADDRESS + 10,
            kind: GapKind::Bytes,
        })
    );
}

/// The cap on the derivation exists for a function whose end nothing states; one whose end
/// the unwind table states is that long, however long that is.
#[test]
fn a_stated_end_beats_the_cap() {
    let mut text = vec![0x90u8; (1 << 20) + 16];
    *text.last_mut().unwrap() = 0xC3;
    let object = parse(&pe_image(PeDll {
        text: &text,
        symbols: &[FIRST],
        entry: None,
        codeview: None,
        unwind: &[(0, text.len() as u64)],
        fragments: &[],
    }));
    let first = named(&object, "first");
    assert_eq!(first.estimate_size(), Some(1 << 20));
    assert_eq!(first.extent(&object), Some((1 << 20) + 16));
    assert_eq!(
        first.data_in(&object).map(<[u8]>::len),
        Some((1 << 20) + 16)
    );
}

/// A symbol inside an entry's range — a label, a public mid-function — is given the rest of
/// the range; and the symbol the range begins at stops at that label, since the listing
/// decodes one stretch per symbol and would otherwise draw the label's rows twice.
#[test]
fn an_entry_covering_a_label_inside_it_is_clamped_to_the_next_symbol() {
    let object = parse(&pe_image(PeDll {
        text: TEXT,
        symbols: &[
            FIRST,
            ExportedSymbol {
                name: "label",
                offset: 2,
                size: 0,
                code: true,
            },
        ],
        entry: None,
        codeview: None,
        unwind: &[(0, 4)],
        fragments: &[],
    }));
    assert_eq!(named(&object, "first").extent(&object), Some(2));
    let label = named(&object, "label");
    assert_eq!(label.estimate_size(), Some(8), "to the section's end");
    assert_eq!(label.extent(&object), Some(2), "to the entry's end");
}

/// An end the table states past the section's bytes is clamped to them as it is read in,
/// so a function it covers still has bytes to decode.
#[test]
fn an_entry_reaching_past_the_section_is_clamped_to_its_bytes() {
    let object = parse(&pe_image(PeDll {
        text: TEXT,
        symbols: &[ExportedSymbol {
            name: "last",
            offset: 7,
            size: 0,
            code: true,
        }],
        entry: None,
        codeview: None,
        unwind: &[(7, 0x800)],
        fragments: &[],
    }));
    let last = named(&object, "last");
    let section = last.section.clone().unwrap();
    assert_eq!(section.unwind, [TEXT_ADDRESS + 7..TEXT_ADDRESS + 10]);
    assert_eq!(last.extent(&object), Some(3));
    assert!(last.assembly(&object).is_some());
}

/// A chained entry — one whose `UNWIND_INFO` carries `UNW_FLAG_CHAININFO`: a cold part, or
/// the piece after a mid-body stack adjustment, of a function with a primary entry
/// elsewhere — is a **fragment**, named `<fragment 0x…>` by its own address as a function
/// is, with its stated length and extent; and the function it continues stops where it
/// begins. A plain entry's unwind info is zeroes here, version 0, which is not chained.
#[test]
fn a_chained_entry_is_a_fragment() {
    let object = parse(&pe_image(PeDll {
        text: TEXT,
        symbols: &[FIRST],
        entry: None,
        codeview: None,
        unwind: &[(0, 4), (7, 10)],
        fragments: &[(4, 6)],
    }));
    assert_eq!(
        names(&object),
        [
            format!("<fragment {:#x}>", TEXT_ADDRESS + 4),
            format!("<function {:#x}>", TEXT_ADDRESS + 7),
            "first".to_owned(),
        ]
    );
    let fragment = named(&object, &format!("<fragment {:#x}>", TEXT_ADDRESS + 4));
    assert_eq!(fragment.size, 2);
    assert_eq!(fragment.extent(&object), Some(2));
    assert_eq!(fragment.demangled, None);
    assert_eq!(named(&object, "first").extent(&object), Some(4));
}

/// A chained entry at an address something names adds nothing, as any entry: the name is
/// the image's, and the range still states the extent.
#[test]
fn a_chained_entry_at_a_named_address_keeps_the_name() {
    let object = parse(&pe_image(PeDll {
        text: TEXT,
        symbols: &[FIRST, SECOND],
        entry: None,
        codeview: None,
        unwind: &[],
        fragments: &[(4, 6)],
    }));
    assert_eq!(names(&object), ["first", "second"]);
    assert_eq!(named(&object, "second").extent(&object), Some(2));
}

/// The in-memory ELF writer's `.eh_frame` reads back through `gimli` as `gcc`'s does: a
/// section by that name, ending in the zero-length terminator, whose FDEs — pc-relative,
/// resolved against the section's own address — state exactly the ranges asked for.
#[test]
fn the_writers_eh_frame_reads_back_through_gimli() {
    use gimli::{BaseAddresses, CieOrFde, EhFrame, LittleEndian, UnwindSection as _};

    let image = elf_shared_object(SharedObject {
        text: TEXT,
        dynamic: &[FIRST],
        static_symbols: &[],
        entry: None,
        eh_frame: &[(0, 4), (4, 6)],
    });
    let file = object::File::parse(image.as_slice()).expect("an ELF image");
    let section = file.section_by_name(".eh_frame").expect("an .eh_frame");
    let data = section.data().expect("its bytes");
    assert_eq!(&data[data.len() - 4..], &[0; 4], "the terminator");

    let eh_frame = EhFrame::new(data, LittleEndian);
    let bases = BaseAddresses::default().set_eh_frame(section.address());
    let mut entries = eh_frame.entries(&bases);
    let mut fdes = Vec::new();
    while let Some(entry) = entries.next().expect("every record parses") {
        if let CieOrFde::Fde(partial) = entry {
            let fde = partial
                .parse(|section, bases, offset| section.cie_from_offset(bases, offset))
                .expect("the FDE parses");
            fdes.push((fde.initial_address(), fde.len()));
        }
    }
    assert_eq!(fdes, [(TEXT_ADDRESS, 4), (TEXT_ADDRESS + 4, 2)]);
}

/// A stripped shared object with FDEs on `first` and on two functions nothing names.
fn shared_with(dynamic: &[ExportedSymbol], entry: Option<u64>, eh_frame: &[(u64, u64)]) -> Vec<u8> {
    elf_shared_object(SharedObject {
        text: TEXT,
        dynamic,
        static_symbols: &[],
        entry,
        eh_frame,
    })
}

/// An ELF's FDEs are unwind entries as a PE's `RUNTIME_FUNCTION`s are: a function only its
/// FDE declares is a `<function 0x…>` by its address, with the FDE's length as its declared
/// size, and it disassembles like any other.
#[test]
fn an_elfs_fdes_are_symbols_where_nothing_names_them() {
    let object = parse(&shared_with(&[FIRST], None, &[(0, 4), (4, 6), (7, 10)]));
    assert_eq!(
        names(&object),
        [
            format!("<function {:#x}>", TEXT_ADDRESS + 4),
            format!("<function {:#x}>", TEXT_ADDRESS + 7),
            "first".to_owned(),
        ]
    );
    let second = named(&object, &format!("<function {:#x}>", TEXT_ADDRESS + 4));
    assert_eq!(second.size, 2, "the FDE's length");
    assert_eq!(second.demangled, None);
    assert_eq!(
        second.section.as_ref().map(|s| s.name.as_str()),
        Some(".text")
    );
    let assembly = second.assembly(&object).expect("it decodes");
    assert_eq!(assembly.instructions.len(), 2, "nop; ret");
}

/// The end an FDE states is the function's, padding excluded, where the next symbol's
/// address is not: the same rule as a PE entry's, on an ELF's own table.
#[test]
fn an_fdes_end_beats_the_next_symbols_address() {
    const TEXT: &[u8] = &[
        0x90, 0x90, 0x90, 0x90, 0x90, 0xC3, // 0: five nops and a ret
        0xCC, 0xCC, 0xCC, 0xCC, // 6: the linker's int3 padding
        0x90, 0xC3, // 10
    ];
    let object = parse(&elf_shared_object(SharedObject {
        text: TEXT,
        dynamic: &[
            FIRST,
            ExportedSymbol {
                name: "second",
                offset: 10,
                size: 0,
                code: true,
            },
        ],
        static_symbols: &[],
        entry: None,
        eh_frame: &[(0, 6), (10, 12)],
    }));
    let first = named(&object, "first");
    assert_eq!(first.estimate_size(), Some(10));
    assert_eq!(first.debug_extent(&object), None, "no debug info at all");
    assert_eq!(first.extent(&object), Some(6));
    assert_eq!(first.assembly(&object).unwrap().instructions.len(), 6);

    let listing = Listing::new(&object, first.section.clone().unwrap());
    let stretch = listing.decode(&object, 0).expect("first decodes");
    assert_eq!(
        stretch.gap,
        Some(Gap {
            range: TEXT_ADDRESS + 6..TEXT_ADDRESS + 10,
            kind: GapKind::Bytes,
        })
    );
}

/// The cap on the derivation, which stayed for an ELF while only a PE's ends were stated,
/// is beaten by an FDE's end the same way.
#[test]
fn an_fdes_end_beats_the_cap() {
    let mut text = vec![0x90u8; (1 << 20) + 16];
    *text.last_mut().unwrap() = 0xC3;
    let object = parse(&elf_shared_object(SharedObject {
        text: &text,
        dynamic: &[FIRST],
        static_symbols: &[],
        entry: None,
        eh_frame: &[(0, text.len() as u64)],
    }));
    let first = named(&object, "first");
    assert_eq!(first.estimate_size(), Some(1 << 20));
    assert_eq!(first.extent(&object), Some((1 << 20) + 16));
}

/// A `.dynsym` function inside an FDE's range is given the rest of it, and the function the
/// FDE begins at stops at that label: the next-symbol clamp, on an ELF.
#[test]
fn an_fde_covering_a_label_inside_it_is_clamped_to_the_next_symbol() {
    let object = parse(&shared_with(
        &[
            FIRST,
            ExportedSymbol {
                name: "label",
                offset: 2,
                size: 0,
                code: true,
            },
        ],
        None,
        &[(0, 4)],
    ));
    assert_eq!(named(&object, "first").extent(&object), Some(2));
    let label = named(&object, "label");
    assert_eq!(label.estimate_size(), Some(8), "to the section's end");
    assert_eq!(label.extent(&object), Some(2), "to the FDE's end");
}

/// An FDE's end past the section's bytes is clamped to them as it is placed.
#[test]
fn an_fde_reaching_past_the_section_is_clamped_to_its_bytes() {
    let object = parse(&shared_with(
        &[ExportedSymbol {
            name: "last",
            offset: 7,
            size: 0,
            code: true,
        }],
        None,
        &[(7, 0x800)],
    ));
    let last = named(&object, "last");
    let section = last.section.clone().unwrap();
    assert_eq!(section.unwind, [TEXT_ADDRESS + 7..TEXT_ADDRESS + 10]);
    assert_eq!(last.extent(&object), Some(3));
    assert!(last.assembly(&object).is_some());
}

/// An FDE at an address something names — a `.dynsym` function, the entry point — adds no
/// second symbol; one of length 0 states nothing, and one whose start is not in code (a
/// page past `.text` is `.data`) is dropped by the section lookup as a PE entry is.
#[test]
fn an_fde_at_a_named_address_adds_no_symbol_and_an_empty_one_nothing() {
    let object = parse(&shared_with(
        &[FIRST, SECOND],
        Some(6),
        &[(0, 4), (4, 6), (6, 7), (4, 4), (0x1000, 0x1004)],
    ));
    assert_eq!(names(&object), ["<entry point>", "first", "second"]);
    assert_eq!(object.symbols.len(), 3);
    let section = named(&object, "first").section.clone().unwrap();
    assert_eq!(
        section.unwind,
        [
            TEXT_ADDRESS..TEXT_ADDRESS + 4,
            TEXT_ADDRESS + 4..TEXT_ADDRESS + 6,
            TEXT_ADDRESS + 6..TEXT_ADDRESS + 7,
        ]
    );
}

/// `.eh_frame` is the same format on every architecture, so an AArch64 ELF's is read as an
/// x86-64's — the opposite of `.pdata`, whose ARM64 record is another shape.
#[test]
fn an_aarch64_elfs_eh_frame_is_read() {
    let mut image = shared_with(&[FIRST], None, &[(0, 4), (4, 6), (7, 10)]);
    // `e_machine`: EM_AARCH64.
    image[18..20].copy_from_slice(&183u16.to_le_bytes());
    let object = parse(&image);
    assert_eq!(object.architecture, Architecture::Aarch64);
    assert_eq!(
        names(&object),
        [
            format!("<function {:#x}>", TEXT_ADDRESS + 4),
            format!("<function {:#x}>", TEXT_ADDRESS + 7),
            "first".to_owned(),
        ]
    );
}

/// A record whose length cannot be trusted ends the walk where it stands: the FDEs before
/// it are kept, nothing after it is guessed at, and nothing panics.
#[test]
fn a_cut_eh_frame_yields_what_parsed_before_the_cut() {
    let mut image = shared_with(&[FIRST], None, &[(4, 6), (7, 10)]);
    let (offset, size) = {
        let file = object::File::parse(image.as_slice()).unwrap();
        let section = file.section_by_name(".eh_frame").unwrap();
        section.file_range().unwrap()
    };
    let start = offset as usize;
    let bytes = &image[start..start + size as usize];
    // The CIE, then the first FDE, then the second: each record's length word says where
    // the next begins.
    let cie_len = u32::from_le_bytes(bytes[0..4].try_into().unwrap()) as usize;
    let fde1 = 4 + cie_len;
    let fde1_len = u32::from_le_bytes(bytes[fde1..fde1 + 4].try_into().unwrap()) as usize;
    let fde2 = fde1 + 4 + fde1_len;
    image[start + fde2..start + fde2 + 4].copy_from_slice(&0xffff_fff0u32.to_le_bytes());

    let object = parse(&image);
    assert_eq!(
        names(&object),
        [
            format!("<function {:#x}>", TEXT_ADDRESS + 4),
            "first".to_owned()
        ]
    );
}

//! Which disassembler decodes a symbol, and what happens when none does.
//!
//! Every fixture is the same shape and differs only in the `e_machine` its header
//! declares: the decoder is a property of the file, and the bytes never say how to read
//! themselves, so a decoder pinned to one width is wrong silently rather than loudly.

mod common;

use analysis::Architecture;
use common::{elf_text, parse, symbol, text, TextSymbol};

/// `48` is `dec eax` in 32-bit code and the REX.W prefix in 64-bit code, so these four
/// bytes are three instructions or two depending on the width they are decoded at.
const AMBIGUOUS_WIDTH: &[u8] = &[0x48, 0x31, 0xC0, 0xC3];

/// `mov x0, #1; ret` in aarch64 — real code for a machine no backend here decodes.
const AARCH64: &[u8] = &[0x20, 0x00, 0x80, 0xD2, 0xC0, 0x03, 0x5F, 0xD6];

fn one_function(architecture: Architecture, bytes: &[u8]) -> Vec<u8> {
    elf_text(architecture, &[TextSymbol { name: "f", bytes }], &[])
}

#[test]
fn x86_64_decodes_as_64_bit() {
    let object = parse(&one_function(Architecture::X86_64, AMBIGUOUS_WIDTH));
    let assembly = symbol(&object, "f")
        .assembly(&object)
        .expect("the symbol has bytes");

    assert_eq!(assembly.undecodable, None);
    let listing: Vec<String> = assembly.instructions.iter().map(text).collect();
    assert_eq!(listing, ["xor       rax, rax", "ret"]);
}

/// A 32-bit object used to be decoded by a decoder built with `with_ip(64, ..)`, which
/// does not fail — it reads the same bytes as a different program.
#[test]
fn x86_32_decodes_as_32_bit() {
    let object = parse(&one_function(Architecture::I386, AMBIGUOUS_WIDTH));
    assert_eq!(object.architecture, Architecture::I386);

    let assembly = symbol(&object, "f")
        .assembly(&object)
        .expect("the symbol has bytes");

    assert_eq!(assembly.undecodable, None);
    let listing: Vec<String> = assembly.instructions.iter().map(text).collect();
    assert_eq!(listing, ["dec       eax", "xor       eax, eax", "ret"]);
}

/// The x32 ABI is 64-bit *code* with 32-bit pointers, so it decodes with the 64-bit
/// decoder even though the file's class says 32 — the one case asking `is_64()` instead
/// of the architecture would get backwards.
#[test]
fn the_x32_abi_decodes_as_64_bit() {
    let object = parse(&one_function(Architecture::X86_64_X32, AMBIGUOUS_WIDTH));
    let assembly = symbol(&object, "f")
        .assembly(&object)
        .expect("the symbol has bytes");

    assert_eq!(assembly.undecodable, None);
    let listing: Vec<String> = assembly.instructions.iter().map(text).collect();
    assert_eq!(listing, ["xor       rax, rax", "ret"]);
}

/// An architecture nothing here decodes is a third answer: no instructions, and the
/// reason they are missing. Not `None` — the symbol has perfectly good bytes.
#[test]
fn an_architecture_with_no_backend_says_so_rather_than_decoding() {
    let object = parse(&one_function(Architecture::Aarch64, AARCH64));
    assert_eq!(object.architecture, Architecture::Aarch64);

    let f = symbol(&object, "f");
    // The bytes are there to decode; only the decoder is missing.
    assert_eq!(f.data_in(&object).map(<[u8]>::len), Some(AARCH64.len()));

    let assembly = f.assembly(&object).expect("the symbol has bytes");
    assert_eq!(assembly.undecodable, Some("aarch64"));
    assert!(assembly.instructions.is_empty());
    assert!(assembly.edges.is_empty());
}

/// Why the answer above has to be *said*: the very same bytes decode into a confident x86
/// listing the moment the header claims x86, so there is no byte sequence to refuse on.
#[test]
fn the_same_bytes_under_an_x86_header_decode_into_nonsense() {
    let object = parse(&one_function(Architecture::X86_64, AARCH64));
    let assembly = symbol(&object, "f")
        .assembly(&object)
        .expect("the symbol has bytes");

    assert_eq!(assembly.undecodable, None);
    assert!(!assembly.instructions.is_empty());
}

/// An architecture with no spelled-out name still answers with a reason rather than
/// silence.
#[test]
fn an_unnamed_architecture_is_still_undecodable() {
    let object = parse(&one_function(Architecture::Avr, &[0x00, 0x00]));
    let assembly = symbol(&object, "f")
        .assembly(&object)
        .expect("the symbol has bytes");

    assert_eq!(assembly.undecodable, Some("an unsupported architecture"));
    assert!(assembly.instructions.is_empty());
}

//! The two lookups over a listing's rows, held against the invariant they search under.

use super::*;

/// A listing of instructions at `addresses`, with nothing named on any of them.
fn listing(addresses: &[u64]) -> Assembly {
    let instructions = addresses
        .iter()
        .map(|&address| Instruction {
            address: SectionAddress::new(address),
            bytes: Vec::new(),
            format: Vec::new(),
            operand: None,
        })
        .collect();

    Assembly {
        instructions,
        edges: Vec::new(),
        undecodable: None,
        range: SectionAddress::new(0)..SectionAddress::new(0x20),
        extent: Extent {
            bytes: 0x20,
            capped: false,
        },
    }
}

/// **An address lands on the instruction holding it**, which is the last one starting at or
/// before it; an address before the first instruction is nobody's.
#[test]
fn an_address_lands_on_the_instruction_holding_it() {
    let assembly = listing(&[0x10, 0x18]);
    let at = |address| assembly.instruction_at(SectionAddress::new(address));

    assert_eq!(at(0x10), Some(0));
    assert_eq!(at(0x14), Some(0), "inside the first");
    assert_eq!(at(0x18), Some(1));
    assert_eq!(at(0x20), Some(1), "past the last");
    assert_eq!(at(0x0), None, "before the listing's first instruction");
    assert_eq!(
        listing(&[]).instruction_at(SectionAddress::new(0x10)),
        None,
        "a listing with no rows"
    );
}

/// **A branch target wants the instruction starting exactly where it points**: one landing
/// mid-instruction has no row of its own, and is an edge `decoded` drops.
#[test]
fn a_target_mid_instruction_starts_nothing() {
    let assembly = listing(&[0x10, 0x18]);
    let starting = |address| assembly.instruction_starting(SectionAddress::new(address));

    assert_eq!(starting(0x10), Some(0));
    assert_eq!(starting(0x18), Some(1));
    assert_eq!(starting(0x14), None, "inside the first");
    assert_eq!(starting(0x20), None, "past the last");
    assert_eq!(starting(0x0), None, "before the first");
}

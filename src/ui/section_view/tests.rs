use super::*;

/// A row of bytes is read in the object's byte order: `addiu sp,sp,-32` stored big-endian,
/// as MIPS does, reads as the word the file states, and the same bytes in a little-endian
/// object read the other way round.
#[test]
fn a_row_of_bytes_is_read_in_the_objects_byte_order() {
    let bytes = [0x27, 0xBD, 0xFF, 0xE0];
    let (mark, big) = dump_line(&bytes, Endianness::Big);
    assert_eq!(mark, "dd");
    assert!(big.starts_with("27BDFFE0 "), "{big}");
    let (_, little) = dump_line(&bytes, Endianness::Little);
    assert!(little.starts_with("E0FFBD27 "), "{little}");
}

/// Every row's characters start in the same column: 15 lone bytes, the widest values a
/// row can have, are padded to the same width as a full row of quadwords.
#[test]
fn every_row_of_bytes_puts_its_characters_in_one_column() {
    let bar = |len: usize| {
        let (_, line) = dump_line(&vec![0xCC; len], Endianness::Little);
        line.find('|').unwrap()
    };
    let full = bar(GAP_BYTES_PER_ROW as usize);
    for len in 1..=GAP_BYTES_PER_ROW as usize {
        assert_eq!(bar(len), full, "{len} bytes");
    }
}

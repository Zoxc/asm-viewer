use super::{read_uint, write_uint};
use gimli::RunTimeEndian::{Big, Little};

/// Both widths in both byte orders, the way a relocation's field is read and patched. A
/// 4-byte field takes the low word of what is written.
#[test]
fn relocation_fields_read_and_write_in_either_byte_order() {
    let mut field = [0u8; 4];
    write_uint(&mut field, Little, 0xaaaa_bbbb_0102_0304);
    assert_eq!(field, [4, 3, 2, 1]);
    assert_eq!(read_uint(&field, Little), 0x0102_0304);

    write_uint(&mut field, Big, 0xaaaa_bbbb_0102_0304);
    assert_eq!(field, [1, 2, 3, 4]);
    assert_eq!(read_uint(&field, Big), 0x0102_0304);

    let mut field = [0u8; 8];
    write_uint(&mut field, Little, 0x0102_0304_0506_0708);
    assert_eq!(field, [8, 7, 6, 5, 4, 3, 2, 1]);
    assert_eq!(read_uint(&field, Little), 0x0102_0304_0506_0708);

    write_uint(&mut field, Big, 0x0102_0304_0506_0708);
    assert_eq!(field, [1, 2, 3, 4, 5, 6, 7, 8]);
    assert_eq!(read_uint(&field, Big), 0x0102_0304_0506_0708);
}

/// Any other width reads as 0 and is left as it was, rather than panicking.
#[test]
fn other_widths_are_neither_read_nor_written() {
    for len in [0, 1, 2, 3, 5, 7, 9] {
        let mut field = vec![0xffu8; len];
        assert_eq!(read_uint(&field, Little), 0);
        assert_eq!(read_uint(&field, Big), 0);
        write_uint(&mut field, Little, 0);
        write_uint(&mut field, Big, 0);
        assert!(field.iter().all(|&b| b == 0xff));
    }
}

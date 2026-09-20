use super::{Bias, PlacedAddress, SectionAddress};

/// What placing an address *is*, against the numbers themselves: the module's own tests are
/// the only place that can say so, a `Bias` being opaque to everyone else. Every other
/// assertion in the crate rests on this one.
#[test]
fn placing_an_address_adds_the_bias_and_taking_it_back_subtracts_it() {
    let address = SectionAddress::new(0x1000);
    assert_eq!(address.placed(Bias::new(0x30)).0, 0x1030);
    assert_eq!(PlacedAddress::new(0x1030).local(Bias::new(0x30)).0, 0x1000);

    // Wrapping both ways, which is what `line::relocate` does with the same bias.
    assert_eq!(SectionAddress::new(1).placed(Bias::new(u64::MAX)).0, 0);
    assert_eq!(PlacedAddress::new(0).local(Bias::new(1)).0, u64::MAX);
}

/// The round trip every listing rests on: a section's address placed and taken back is the
/// address again, **whatever the bias**. Wrapping both ways is what makes that hold for a
/// bias the layout could not have produced, which is the case the checked conversions
/// decline rather than answer wrongly.
#[test]
fn placing_an_address_and_taking_it_back_is_the_address() {
    for bias in [0, 1, 16, 0x1000, u64::MAX].map(Bias::new) {
        for address in [0, 1, 0x40, u64::MAX] {
            let address = SectionAddress::new(address);
            assert_eq!(address.placed(bias).local(bias), address, "{bias:?}");
        }
    }
}

/// The checked conversions answer nothing where the wrapping ones wrap, and the same thing
/// as those wherever they answer at all. A caller that must not ask about a different
/// address than it meant takes the first behaviour; `line::relocate`'s agreement rests on
/// the second.
#[test]
fn the_checked_conversions_agree_where_they_answer() {
    let address = SectionAddress::new(0x1000);
    let small = Bias::new(0x10);
    let huge = Bias::new(u64::MAX);
    assert_eq!(address.placed_checked(small), Some(address.placed(small)));
    assert_eq!(address.placed_checked(huge), None);
    assert_eq!(
        address.placed_saturating(huge),
        PlacedAddress::new(u64::MAX)
    );

    let placed = PlacedAddress::new(0x1000);
    assert_eq!(placed.local_checked(small), Some(placed.local(small)));
    assert_eq!(placed.local_checked(Bias::new(0x2000)), None);

    // Nothing placed it, so the number is the same in both spaces.
    assert_eq!(address.unplaced(), PlacedAddress::new(0x1000));
    assert_eq!(address.unplaced(), address.placed(Bias::NONE));
}

/// The bytes between two addresses are a length and not an address, and a range stated
/// backwards has none. The saturating half answers 0 there instead, for a caller counting
/// rows rather than asking a question.
#[test]
fn the_bytes_between_two_addresses_are_none_when_they_run_backwards() {
    let (low, high) = (SectionAddress::new(0x10), SectionAddress::new(0x18));

    assert_eq!(low.bytes_to(high), Some(8));
    assert_eq!(low.bytes_to(low), Some(0));
    assert_eq!(high.bytes_to(low), None);

    assert_eq!(low.bytes_to_saturating(high), 8);
    assert_eq!(high.bytes_to_saturating(low), 0);
}

/// Every step over a number out of a file is checked, at both ends of the space.
#[test]
fn stepping_off_either_end_of_the_address_space_answers_nothing() {
    let top = SectionAddress::new(u64::MAX);
    assert_eq!(top.checked_add(1), None);
    assert_eq!(top.saturating_add(1), top);
    assert_eq!(top.checked_add(0), Some(top));

    let bottom = SectionAddress::new(0);
    assert_eq!(bottom.checked_sub(1), None);
    assert_eq!(bottom.checked_sub(0), Some(bottom));
}

/// A failing assertion has to say which space it was about, so the two spell themselves
/// apart; and hex is what an address is written in, with the caller's own flags.
#[test]
fn an_address_says_its_space_and_prints_as_hex() {
    let address = SectionAddress::new(0x1234);
    assert_eq!(format!("{address:?}"), "SectionAddress(0x1234)");
    assert_eq!(
        format!("{:?}", PlacedAddress::new(0x1234)),
        "PlacedAddress(0x1234)"
    );

    assert_eq!(format!("{address:#x}"), "0x1234");
    assert_eq!(format!("{address:08X}"), "00001234");
}

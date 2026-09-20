//! The two address spaces the crate reads in, each a type of its own: [`SectionAddress`],
//! one of a section's own, and [`PlacedAddress`], the one space every section of an object
//! shares. **The conversion between them is here and nowhere else** —
//! [`Section::place`](crate::Section::place) and [`Section::local`](crate::Section::local)
//! are these with a section's bias handed in.
//!
//! Two spaces a bias apart, both `u64`, was easy to get wrong: a number said nothing about
//! which space it was in, and a forgotten bias is invisible on a linked image, where every
//! bias is 0, and wrong on every relocatable object. So the space is in the type, and the
//! arithmetic an address is allowed is what a listing does with one — a length added, the
//! bytes between two — rather than everything a `u64` can do.
//!
//! **A section is not in the type**, so two of one section's addresses and two of another's
//! compare alike; only the space is checked.

use std::fmt;

/// An address in one section's own terms: what a file states for its sections and its
/// symbols, what a symbol's listing draws, and what the bytes of a section are sliced by.
///
/// In a linked image these are the real addresses. In a relocatable object every code
/// section starts at 0, so one of these alone does not say which code it is: that is
/// [`PlacedAddress`]'s job, and [`Section::place`](crate::Section::place) is the way across.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SectionAddress(u64);

/// An address in the one space every section of an object shares: a section's own address
/// with that section's [`bias`](crate::CodeSection::bias) added.
///
/// What [`Object::symbol_at_placed`](crate::Object::symbol_at_placed) answers in, what a
/// listing of a whole object's code draws in, and what the debug info is read in
/// ([`section_biases`](crate::parse::section_biases)).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PlacedAddress(u64);

/// What a section's own addresses are moved by to place them: what
/// [`section_biases`](crate::parse::section_biases) gives each code section of a relocatable
/// object, and nothing at all for every other file and for every section holding no code.
///
/// A type rather than a `u64` so that the conversion between the two spaces cannot be handed
/// a length, an offset or a size: the spaces say which space a number is in, and this says
/// that what crosses between them is the thing the layout moved a section by.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct Bias(u64);

impl Bias {
    /// Nothing moved this section: every section of a file that is not a relocatable object,
    /// and every section holding no code, which has no place in the layout at all.
    pub const NONE: Bias = Bias(0);

    /// The number the layout worked out, as a bias. There is no way back to a number:
    /// a bias exists to be added to an address and for nothing else.
    pub const fn new(bias: u64) -> Bias {
        Bias(bias)
    }
}

/// What both spaces can do, written once: the two are the same integer under different
/// meanings, so neither gets an operation the other is denied.
macro_rules! address {
    ($name:ident) => {
        impl $name {
            /// This number as an address in this space: what a file states, and what a
            /// test writes.
            pub const fn new(address: u64) -> $name {
                $name(address)
            }

            /// The plain number back, for whatever does not count in addresses: printing
            /// one, and handing one to a library that takes a `u64`.
            pub const fn get(self) -> u64 {
                self.0
            }

            /// `bytes` further on, or [`None`] where that runs off the end of the address
            /// space. Checked, like every step over a number out of a file.
            pub fn checked_add(self, bytes: u64) -> Option<$name> {
                self.0.checked_add(bytes).map($name)
            }

            /// `bytes` back, or [`None`] where that runs off the bottom of the address
            /// space.
            pub fn checked_sub(self, bytes: u64) -> Option<$name> {
                self.0.checked_sub(bytes).map($name)
            }

            /// `bytes` further on, stopping at the end of the address space: for a row
            /// that draws whatever bytes are left rather than none.
            pub fn saturating_add(self, bytes: u64) -> $name {
                $name(self.0.saturating_add(bytes))
            }

            /// How many bytes from here up to `end`, or [`None`] where `end` is below this
            /// address. The one subtraction two addresses make, and it is a length and not
            /// an address.
            pub fn bytes_to(self, end: $name) -> Option<u64> {
                end.0.checked_sub(self.0)
            }

            /// [`bytes_to`](Self::bytes_to) saturating: a range stated backwards is no
            /// bytes rather than nothing, for a caller counting what it has to draw.
            pub fn bytes_to_saturating(self, end: $name) -> u64 {
                end.0.saturating_sub(self.0)
            }
        }

        /// The space as well as the number, so a test that fails says which one it was
        /// about.
        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}({:#x})", stringify!($name), self.0)
            }
        }

        /// Hex as the number itself formats, so a caller's width, sign and `#` all hold:
        /// an address is written in hex wherever it is shown.
        impl fmt::LowerHex for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                fmt::LowerHex::fmt(&self.0, f)
            }
        }

        impl fmt::UpperHex for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                fmt::UpperHex::fmt(&self.0, f)
            }
        }
    };
}

address!(SectionAddress);
address!(PlacedAddress);

impl SectionAddress {
    /// This address in the object's one space, `bias` being the section's
    /// ([`Section::bias`](crate::Section::bias)).
    ///
    /// `wrapping_add` and not `checked_add`, as `line::relocate` adds the same bias:
    /// agreeing with it matters more than an overflow the biases cannot produce, the layout
    /// starting above the highest address the file states
    /// ([`section_biases`](crate::parse::section_biases)). Wrapping is also what keeps this
    /// from panicking on an address a file made up.
    pub fn placed(self, bias: Bias) -> PlacedAddress {
        PlacedAddress(self.0.wrapping_add(bias.0))
    }

    /// [`placed`](Self::placed) with the overflow said, for a caller that must answer
    /// nothing rather than answer about a different address. A bias is never a wrapped
    /// value, so the two agree wherever this one answers.
    pub(crate) fn placed_checked(self, bias: Bias) -> Option<PlacedAddress> {
        self.0.checked_add(bias.0).map(PlacedAddress)
    }

    /// [`placed`](Self::placed) saturating, for the ends of a query: an absurd range then
    /// asks about less than it meant to instead of about something else.
    pub(crate) fn placed_saturating(self, bias: Bias) -> PlacedAddress {
        PlacedAddress(self.0.saturating_add(bias.0))
    }

    /// This address where **nothing placed it**: [`placed`](Self::placed) by [`Bias::NONE`],
    /// which is the same number in the other space.
    ///
    /// Two cases, and they are the whole of it: a file with no layout, which is every file
    /// that is not a relocatable object, since `section_biases` gives out biases for those
    /// alone; and a symbol in no section, which is in no listing to be placed in. Named, so
    /// that a caller says which of those it is standing on rather than passing a bias it
    /// does not have.
    pub(crate) fn unplaced(self) -> PlacedAddress {
        self.placed(Bias::NONE)
    }
}

impl PlacedAddress {
    /// This address back in the terms of the section `bias` placed it from:
    /// [`SectionAddress::placed`] undone, and wrapping for the same reason.
    pub fn local(self, bias: Bias) -> SectionAddress {
        SectionAddress(self.0.wrapping_sub(bias.0))
    }

    /// [`local`](Self::local) with the underflow said, for a caller that must answer
    /// nothing rather than answer about a different address.
    pub(crate) fn local_checked(self, bias: Bias) -> Option<SectionAddress> {
        self.0.checked_sub(bias.0).map(SectionAddress)
    }
}

#[cfg(test)]
mod tests;

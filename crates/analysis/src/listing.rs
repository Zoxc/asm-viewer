//! A whole section as one listing, keyed by address: every symbol's instructions in address
//! order, the symbol as a label where it starts, and the bytes between one symbol's extent
//! and the next symbol — padding, data, code no symbol claims — shown as bytes.
//!
//! **The skeleton is free and the decoding is not.** A [`Listing`] is built from the symbol
//! addresses alone ([`Object::placed`]), one [`Stretch`] per address, and decodes nothing: x86
//! is variable-length, so instruction *n* of a section cannot be found without decoding from a
//! known start, and the symbol addresses are those starts. A stretch is decoded on
//! demand ([`Listing::decode`]), which is when the symbol's extent is asked for and the
//! stretch's trailing [`Gap`] is known — a DWARF walk, for a symbol no unwind entry covers,
//! and the pass over a whole section was seconds on a large binary before the unwind tables
//! were read, which is not "up front".
//!
//! **A gap is never decoded.** Bytes no symbol claims are not known to be code, and decoding
//! them would print a confident page of nonsense over a jump table or a run of padding — the
//! same lesson [`Assembly::undecodable`] records for a foreign architecture. A gap is *said*,
//! as its range and its kind, and whoever draws it asks [`Section::bytes_in`] for the bytes.
//!
//! Nothing here is cached: the listing is a pure function of the object, and whoever asks
//! holds the answer.

use crate::model::covering;
use crate::{Assembly, Bias, Object, PlacedAddress, Section, SectionAddress, SymbolData};
use std::{ops::Range, sync::Arc};

/// One section's listing: its stretches, contiguous and in address order, partitioning the
/// section's bytes exactly.
pub struct Listing {
    section: Arc<Section>,
    stretches: Vec<Stretch>,
}

/// One label's worth of a section: the bytes from a symbol's address up to the next
/// symbol's, or the bytes before the first symbol.
pub struct Stretch {
    /// The addresses this stretch covers, `start` being where its label sits. Ends where the
    /// next stretch starts, or at the section's end.
    pub range: Range<SectionAddress>,

    /// The symbols at `range.start`, in the order the file's symbol table has them. Empty
    /// for the leading stretch — the bytes before the first symbol, or a section with no
    /// symbols at all — and more than one where the file has two names for one address.
    pub symbols: Vec<Arc<SymbolData>>,
}

impl Stretch {
    /// The symbol whose code this stretch holds, if it holds any: the first of the names at
    /// its address. Every name at one address decodes the same bytes.
    pub fn symbol(&self) -> Option<&Arc<SymbolData>> {
        self.symbols.first()
    }
}

/// What decoding one stretch yields: the symbol's own instructions, and the bytes left
/// between its extent and the next label.
pub struct DecodedStretch {
    /// The symbol's listing, exactly what [`SymbolData::assembly`] answers for it; [`None`]
    /// for the leading stretch, and for a symbol with no bytes to decode.
    pub code: Option<Arc<Assembly>>,

    /// The bytes past the symbol's extent, up to the next label. [`None`] when the extent
    /// reaches the next label exactly.
    pub gap: Option<Gap>,
}

/// A run of bytes no symbol's extent claims. Its bytes are the section's — `range` asked of
/// [`Section::bytes_in`] — and are not copied here, since a gap can be megabytes.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Gap {
    pub range: Range<SectionAddress>,
    pub kind: GapKind,
}

/// What a gap is, as far as this crate can say.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GapKind {
    /// Bytes between one symbol's extent and the next symbol, or before the first: alignment
    /// padding, a jump table, code nothing names. Which of those is not known and none of
    /// them is decoded.
    Bytes,

    /// The rest of a stretch whose symbol's extent was capped
    /// ([`Extent::capped`](crate::Extent::capped)): very likely the symbol's own code going
    /// on past the cap rather than anything between two functions, and starting wherever the
    /// cap fell rather than at an instruction. Said
    /// apart so a reader is not told the function ends there. Where the file has an unwind
    /// table only a symbol no entry covers can get here; the rest have their ends stated.
    Cut,
}

impl Listing {
    /// The skeleton for `section`: one stretch per distinct symbol address inside its bytes,
    /// plus a leading one when the first symbol is not at its start. Decodes nothing.
    ///
    /// A symbol placed outside the section's bytes — a wild `st_value` — is left out; a
    /// section with no bytes has no stretches, and so does one placed at the very end of
    /// the address space, whose bytes have no addresses to be at.
    pub fn new(object: &Object, section: Arc<Section>) -> Self {
        let Some(bytes) = section.bytes_range() else {
            return Self {
                section,
                stretches: Vec::new(),
            };
        };
        // The object's symbols over the section's placed range, already sorted by address
        // and then by index: two sections of a relocatable object share address 0, and
        // their places do not. Each placed address less the bias is the address in the
        // section, which for the section's own symbols is the address they state.
        let symbols = section
            .placed_range()
            .map_or(&[][..], |range| object.placed_in(range));
        let local = |&(placed, ..): &(PlacedAddress, _, _)| section.local(placed);

        let mut stretches = Vec::new();
        let first = symbols.first().map_or(bytes.end, local);
        if bytes.start < first {
            stretches.push(Stretch {
                range: bytes.start..first,
                symbols: Vec::new(),
            });
        }

        let mut rest = symbols;
        while let Some(address) = rest.first().map(local) {
            let count = rest
                .iter()
                .take_while(|entry| local(entry) == address)
                .count();
            let (here, after) = rest.split_at(count);
            let next = after.first().map_or(bytes.end, local);
            stretches.push(Stretch {
                range: address..next,
                symbols: here.iter().map(|(_, _, symbol)| symbol.clone()).collect(),
            });
            rest = after;
        }

        Self { section, stretches }
    }

    pub fn section(&self) -> &Arc<Section> {
        &self.section
    }

    /// The stretches, in address order, partitioning the section's bytes.
    pub fn stretches(&self) -> &[Stretch] {
        &self.stretches
    }

    /// The index of the stretch `address` falls in, if it is in the section.
    pub fn stretch_at(&self, address: SectionAddress) -> Option<usize> {
        covering(&self.stretches, |stretch| stretch.range.clone(), address)
    }

    /// Decode the stretch at `index`: the symbol's own instructions, and whatever the
    /// symbol's extent leaves before the next label as a [`Gap`]. [`None`] only for an index
    /// the listing has no stretch at.
    ///
    /// The code is [`SymbolData::assembly`]'s answer, literally, so the section listing and
    /// the symbol's own cannot disagree; where the gap starts is read off that answer
    /// ([`Assembly::range`]) rather than decided again, [`SymbolData::extent`] being the
    /// most expensive answer in the crate. A symbol whose bytes would not read has no
    /// assembly and no range either, so its whole stretch is a gap.
    pub fn decode(&self, object: &Object, index: usize) -> Option<DecodedStretch> {
        let stretch = self.stretches.get(index)?;
        let Some(symbol) = stretch.symbol() else {
            return Some(DecodedStretch {
                code: None,
                gap: Some(Gap {
                    range: stretch.range.clone(),
                    kind: GapKind::Bytes,
                }),
            });
        };

        let code = symbol.assembly(object);
        // Clipped: the extent is bounded by the next symbol already, but with no next symbol
        // and a section reaching the end of the address space it is the file's number.
        let claimed = code.as_ref().map_or(stretch.range.start, |code| {
            code.range.end.min(stretch.range.end)
        });
        // The extent says whether it was capped, so an end the file states as exactly the
        // cap is not mistaken for one.
        let gap = (claimed < stretch.range.end).then(|| Gap {
            range: claimed..stretch.range.end,
            kind: match code.as_ref().is_some_and(|code| code.extent.capped) {
                true => GapKind::Cut,
                false => GapKind::Bytes,
            },
        });

        Some(DecodedStretch { code, gap })
    }
}

/// Every code section of one object as one listing, in the one address space the parse laid
/// them out in: what a reader scrolling "all the code" scrolls.
///
/// Each section keeps its own [`Listing`] and is **placed** at
/// [`CodeSection::bias`](crate::CodeSection::bias) past its own address. A linked image's sections already sit at distinct addresses and have no
/// bias, so a placed address is the address; a relocatable object's code sections all start
/// at 0 and the parse gave each a place of its own (`section_biases`), the same one its line
/// info is read at. The air the layout leaves between two sections is nothing's bytes and is
/// not a gap: [`at`](Self::at) answers [`None`] there.
///
/// Sections are in placed order. A section whose bytes have no place is left out: one holding
/// no code, one with no bytes (gcc leaves an empty `.text` beside its `.text.<name>`s), and
/// one that does not fit in the address space. So is one whose placed range overlaps the
/// section before it, which a file's headers can claim but nothing can draw.
pub struct CodeListing {
    sections: Vec<Placed>,
}

/// One code section in a [`CodeListing`]: its listing, and where the layout put it.
pub struct Placed {
    pub listing: Listing,
    range: Range<PlacedAddress>,
}

impl Placed {
    /// What is added to an address in this section to place it.
    pub fn bias(&self) -> Bias {
        self.listing.section().bias()
    }

    /// The placed addresses this section's bytes occupy.
    pub fn range(&self) -> Range<PlacedAddress> {
        self.range.clone()
    }

    /// The placed address of an address in this section ([`Section::place`]).
    pub fn place(&self, address: SectionAddress) -> PlacedAddress {
        self.listing.section().place(address)
    }

    /// The address in this section of a placed address ([`Section::local`]).
    pub fn local(&self, placed: PlacedAddress) -> SectionAddress {
        self.listing.section().local(placed)
    }
}

/// A place in a [`CodeListing`]: which of its sections, and which stretch of that.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Place {
    pub section: usize,
    pub stretch: usize,
}

impl CodeListing {
    /// The skeleton of every code section, decoding nothing: each section's [`Listing`]
    /// over its own run of [`Object::placed`].
    pub fn new(object: &Object) -> Self {
        let mut placed: Vec<Placed> = object
            .sections
            .iter()
            .filter_map(|section| {
                let range = section.placed_range()?;
                Some(Placed {
                    listing: Listing::new(object, section.clone()),
                    range,
                })
            })
            .collect();
        placed.sort_by_key(|placed| (placed.range.start, placed.listing.section().index.0));

        let mut sections: Vec<Placed> = Vec::with_capacity(placed.len());
        for next in placed {
            if sections
                .last()
                .is_none_or(|last| last.range.end <= next.range.start)
            {
                sections.push(next);
            }
        }
        Self { sections }
    }

    /// The code sections, in placed order.
    pub fn sections(&self) -> &[Placed] {
        &self.sections
    }

    /// Which of [`sections`](Self::sections) is `section`, by identity; [`None`] for one
    /// that is not code, has no bytes, or was dropped for overlapping.
    pub fn section_of(&self, section: &Section) -> Option<usize> {
        self.sections
            .iter()
            .position(|placed| std::ptr::eq(&**placed.listing.section(), section))
    }

    /// Where a placed address is: the section it falls in and the stretch of that section.
    /// [`None`] between two sections, and outside every one ([`covering`]).
    pub fn at(&self, placed: PlacedAddress) -> Option<Place> {
        let section = covering(&self.sections, |section| section.range.clone(), placed)?;
        let found = &self.sections[section];
        let stretch = found.listing.stretch_at(found.local(placed))?;
        Some(Place { section, stretch })
    }

    /// [`Listing::decode`] for the stretch at `place`.
    pub fn decode(&self, object: &Object, place: Place) -> Option<DecodedStretch> {
        self.sections
            .get(place.section)?
            .listing
            .decode(object, place.stretch)
    }
}

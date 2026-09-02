//! A whole section as one listing, keyed by address: every symbol's instructions in address
//! order, the symbol as a label where it starts, and the bytes between one symbol's extent
//! and the next symbol — padding, data, code no symbol claims — shown as bytes.
//!
//! **The skeleton is free and the decoding is not.** A [`Listing`] is built from the
//! section's sorted symbol addresses alone, one [`Stretch`] per address, and decodes nothing:
//! x86 is variable-length, so instruction *n* of a section cannot be found without decoding
//! from a known start, and the symbol addresses are those starts. A stretch is decoded on
//! demand ([`Listing::decode`]), which is when the symbol's DWARF extent is asked for and the
//! stretch's trailing [`Gap`] is known — the extent pass over a whole section is seconds on
//! a large binary, which is not "up front".
//!
//! **A gap is never decoded.** Bytes no symbol claims are not known to be code, and decoding
//! them would print a confident page of nonsense over a jump table or a run of padding — the
//! same lesson [`Assembly::undecodable`] records for a foreign architecture. A gap is *said*,
//! as its range and its kind, and whoever draws it slices the section's bytes.
//!
//! Nothing here is cached: the listing is a pure function of the object, and whoever asks
//! holds the answer.

use crate::{Assembly, Object, Section, SymbolData, MAX_DERIVED_SIZE};
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
    pub range: Range<u64>,

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

/// A run of bytes no symbol's extent claims. Its bytes are the section's — `range` sliced
/// out of [`Section::data`] — and are not copied here, since a gap can be megabytes.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Gap {
    pub range: Range<u64>,
    pub kind: GapKind,
}

/// What a gap is, as far as this crate can say.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GapKind {
    /// Bytes between one symbol's extent and the next symbol, or before the first: alignment
    /// padding, a jump table, code nothing names. Which of those is not known and none of
    /// them is decoded.
    Bytes,

    /// The rest of a stretch whose symbol's derived extent hit [`MAX_DERIVED_SIZE`]: very
    /// likely the symbol's own code going on past the cap rather than anything between two
    /// functions, and starting wherever the cap fell rather than at an instruction. Said
    /// apart so a reader is not told the function ends there.
    Cut,
}

impl Listing {
    /// The skeleton for `section`: one stretch per distinct symbol address inside its bytes,
    /// plus a leading one when the first symbol is not at its start. Decodes nothing.
    ///
    /// A symbol placed outside the section's bytes — a wild `st_value` — is left out; a
    /// section with no bytes has no stretches.
    pub fn new(object: &Object, section: Arc<Section>) -> Self {
        let end = section_end(&section);

        // The object's symbols in this section, by pointer identity: two sections of a
        // relocatable object share address 0, so an address alone says nothing.
        let mut in_section: Vec<(u64, usize, &Arc<SymbolData>)> = object
            .symbols
            .iter()
            .filter(|(_, symbol)| {
                symbol
                    .section
                    .as_ref()
                    .is_some_and(|own| Arc::ptr_eq(own, &section))
            })
            .filter(|(_, symbol)| section.address <= symbol.address && symbol.address < end)
            .map(|(index, symbol)| (symbol.address, index.0, symbol))
            .collect();
        // The map's order is the hash seed's; the file's is the symbol index.
        in_section.sort_unstable_by_key(|&(address, index, _)| (address, index));

        let mut stretches = Vec::new();
        let first = in_section.first().map_or(end, |&(address, ..)| address);
        if section.address < first {
            stretches.push(Stretch {
                range: section.address..first,
                symbols: Vec::new(),
            });
        }

        let mut rest = in_section.as_slice();
        while let Some(&(address, ..)) = rest.first() {
            let count = rest
                .iter()
                .take_while(|&&(other, ..)| other == address)
                .count();
            let (here, after) = rest.split_at(count);
            let next = after.first().map_or(end, |&(next, ..)| next);
            stretches.push(Stretch {
                range: address..next,
                symbols: here.iter().map(|&(_, _, symbol)| symbol.clone()).collect(),
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
    pub fn stretch_at(&self, address: u64) -> Option<usize> {
        let after = self
            .stretches
            .partition_point(|stretch| stretch.range.start <= address);
        let index = after.checked_sub(1)?;
        self.stretches[index]
            .range
            .contains(&address)
            .then_some(index)
    }

    /// Decode the stretch at `index`: the symbol's own instructions, and whatever the
    /// symbol's extent leaves before the next label as a [`Gap`]. [`None`] only for an index
    /// the listing has no stretch at.
    ///
    /// The code is [`SymbolData::assembly`]'s answer, literally, so the section listing and
    /// the symbol's own cannot disagree; the extent is [`SymbolData::extent`], DWARF-trimmed
    /// where the object has DWARF.
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
        let extent = symbol.extent(object).unwrap_or(0);
        // Saturating, then clipped: the extent is bounded by the next symbol already, but
        // with no next symbol and a section reaching the end of the address space it is the
        // file's number and the sum can wrap.
        let claimed = stretch
            .range
            .start
            .saturating_add(extent)
            .min(stretch.range.end);
        let gap = (claimed < stretch.range.end).then(|| Gap {
            range: claimed..stretch.range.end,
            kind: if extent == MAX_DERIVED_SIZE {
                GapKind::Cut
            } else {
                GapKind::Bytes
            },
        });

        Some(DecodedStretch { code, gap })
    }
}

/// Where the section's bytes stop. Saturating rather than [`None`] like `estimate_size`'s:
/// a listing has to end somewhere, and a section placed so near the end of the address space
/// that it does not fit ends at the end of it.
fn section_end(section: &Section) -> u64 {
    let length: u64 = section.data.len().try_into().unwrap_or(u64::MAX);
    section.address.saturating_add(length)
}

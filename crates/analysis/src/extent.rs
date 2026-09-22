//! How many bytes of code a symbol is ([`SymbolData::extent`]), and the estimate it falls
//! back on. Three answers, in order:
//!
//! 1. **The end the file's own unwind table states**, where an entry covers the address
//!    ([`CodeSection::unwind`](crate::CodeSection::unwind)). Neither the estimate nor its
//!    cap bounds it, and the debug info is not asked.
//! 2. **Then the size the file declares**, where the format makes that a function's length:
//!    an ELF `st_size`, and no other format's.
//! 3. **Else the smaller** of the extent the debug info declares for the function and
//!    [`SymbolData::estimate_size`], the bytes to the next symbol or the section's end. Each
//!    bounds the other in a case the other gets wrong.
//!
//! A length the file states is clamped to the next symbol, since a listing decodes one
//! stretch per symbol. Only the estimate is capped, at [`MAX_DERIVED_SIZE`], and it says so
//! ([`Extent::capped`]). An extent running off the end of the address space is no extent.
//! The answer is worked out at most once per symbol ([`ExtentCache`]).

use crate::model::covering;
use crate::{Object, SymbolData};
use object::BinaryFormat;
use std::ops::Range;
use std::sync::OnceLock;

/// How far [`SymbolData::estimate_size`]'s next-symbol derivation may reach (1 MiB) before
/// it is treated as having said nothing. Not a claim about how long a function can be — five
/// times the largest in any sample here — but the point past which a sparse export table's
/// derivation is certainly describing something else, at megabytes of decoding per redraw.
/// Where the unwind table states where a function ends — an x86-64 PE, an ELF with an
/// `.eh_frame` — the cap reaches only a symbol no entry covers; it stays for the rest.
const MAX_DERIVED_SIZE: u64 = 1 << 20;

/// How many bytes of code a symbol is, and whether that number is where the derivation was
/// capped rather than where the symbol ends. An extent the file states — an unwind entry's
/// end, an ELF `st_size`, the debug info's — is never capped, whatever its value.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Extent {
    pub bytes: u64,

    /// The next-symbol derivation ran past [`MAX_DERIVED_SIZE`] and stops there, so the
    /// bytes past it are very likely the same symbol's code: not where the symbol ends but
    /// where the derivation stopped saying.
    pub capped: bool,
}

/// [`SymbolData::extent`]'s answer, worked out at most once. The *absence* is kept too,
/// the way [`DebugInfoCache`](crate::line::DebugInfoCache) keeps its own: a symbol nothing
/// states an extent for is exactly the one whose answer cost a walk of the debug info.
///
/// It is kept for the **section view**, which drops a decoded stretch once the reader has
/// scrolled well past it and decodes it again on the way back (`src/ui/reading.rs`); the
/// first draw of a symbol asks once either way. Sound because the answer is a function of
/// the file's own tables, and because a `SymbolData` belongs to one object. Asking with
/// another object is a caller's bug, and the memo does not catch it.
#[derive(Debug, Default)]
pub(crate) struct ExtentCache(OnceLock<Option<Extent>>);

impl SymbolData {
    /// Object files frequently state no size, so derive the extent from the next symbol
    /// in the section (or the section end). An *upper* bound rather than a measurement: it
    /// includes alignment padding, and a declaration the symbol table never mentioned (an
    /// export, an entry point) has no size of its own. A derivation running past
    /// [`MAX_DERIVED_SIZE`] stops there and says so ([`Extent::capped`]). See
    /// [`extent`](Self::extent).
    pub fn estimate_size(&self, object: &Object) -> Option<Extent> {
        self.derived(object).map(cap)
    }

    /// [`estimate_size`](Self::estimate_size) before its cap: the bytes from this symbol to
    /// the next in the section, or to the section's end. [`None`] for a symbol outside every
    /// code section's bytes ([`SymbolData::code_place`]). Never 0: the symbol is inside those
    /// bytes, and the next symbol and the section's end are both past it.
    ///
    /// **Every address here is a placed one** ([`SymbolData::placed`]), the symbol's own
    /// included, because the index it reads is: two spaces in one derivation would each
    /// have to be spotted by eye, and a bias forgotten between them is invisible on a
    /// linked image and wrong on every relocatable object. Only the answer leaves, and a
    /// count of bytes is the same number in either space.
    fn derived(&self, object: &Object) -> Option<u64> {
        let range = self.section.as_ref()?.placed_range()?;
        let placed = self.place_in(&range)?;

        // The next symbol is the first entry at a greater address, so a second name at this
        // one bounds nothing, and it counts only inside this section's bytes: past them, the
        // section's end is the bound. A wild address in the symbol table is in no entry, so it
        // cannot cut short the symbol before it.
        let all = object.placed_symbols();
        let after = all.partition_point(|entry| entry.placed <= placed);
        let next = all
            .get(after)
            .map(|entry| entry.placed)
            .filter(|next| range.contains(next));

        placed.bytes_to(next.unwrap_or(range.end))
    }

    /// The end the file's own unwind table states for the function this symbol is in
    /// ([`CodeSection::unwind`](crate::CodeSection::unwind)), as bytes from the symbol's
    /// address, or [`None`] where the entry the address falls in ([`covering`]) states none.
    /// Not yet clamped to the next symbol: [`stated_extent`](Self::stated_extent) does that.
    /// Every entry's own begin is a symbol, so the clamp is what stops a parent at the
    /// chained entry of its cold part.
    fn unwind_extent(&self) -> Option<u64> {
        let code = self.section.as_ref()?.code()?;
        let index = covering(&code.unwind, Range::clone, self.address)?;
        let range = &code.unwind[index];
        self.address.bytes_to(range.end)
    }

    /// The size the file declares for this symbol ([`size`](Self::size)) where that
    /// declaration is a function's length in bytes, not yet clamped to the next symbol;
    /// [`None`] where it declares none or the format's size field is something else.
    ///
    /// **ELF only, and that is an allowlist a format joins on evidence.** An ELF `st_size`
    /// is the ABI's own statement of how many bytes the symbol is, and every mainstream
    /// toolchain fills it in: on `librustc_driver.so` it equals the FDE's length for every
    /// one of the 172 169 functions the `.eh_frame` covers. No other format's size means
    /// that. A COFF function symbol's is the `TotalSize` of an auxiliary
    /// function-definition record, written for COFF's line-number data rather than to
    /// measure code; XCOFF's is a csect's length, and one csect can hold several functions;
    /// Mach-O states no size at all. A declaration that is *wrong* rather than absent would be
    /// taken as fact here, which is why only the field with the measurement behind it is
    /// read.
    ///
    /// The clamp in [`stated_extent`](Self::stated_extent) catches an over-reaching one — hand-written assembly with a `.size` past
    /// the next label. One that is too small is taken as it stands, as an unwind entry's
    /// stated end and a `DW_AT_high_pc` already are.
    fn declared_extent(&self, format: BinaryFormat) -> Option<u64> {
        self.size.filter(|_| format == BinaryFormat::Elf)
    }

    /// How many bytes of code this symbol is. Three answers, in order.
    ///
    /// **The end the file's own unwind table states**, where an entry covers the address
    /// ([`unwind_extent`](Self::unwind_extent)): the image's statement, to its loader, of
    /// the very bytes the unwinder covers, so neither the estimate nor its cap bounds it —
    /// only the next symbol does, for the listing's sake — and the debug info is not asked.
    /// On an x86-64 PE or an ELF with an `.eh_frame` that is nearly every function.
    ///
    /// **Then the size the file declares**, where the format makes that a function's length
    /// ([`declared_extent`](Self::declared_extent)): the symbol table's own answer, which
    /// spares the debug info a walk that would only agree with it. On an ELF built without
    /// unwind tables that is every function its symbol table sizes.
    ///
    /// **Else the smaller** of the extent the debug info declares for the function (DWARF's
    /// `DW_AT_low_pc`/`DW_AT_high_pc`, a PDB procedure's length) and
    /// [`estimate_size`](Self::estimate_size), because each bounds the other in a case the
    /// other gets wrong. The estimate over-reaches into padding and over a function with no
    /// symbol; the declared extent over-reaches when two symbols share one function (an
    /// alias, an assembler label, a split cold part), since it describes the *function*.
    ///
    /// Whichever answers, an extent running off the end of the address space is no extent:
    /// a table stating one describes a range that does not exist, and every caller here
    /// reads `address..address + extent`.
    ///
    /// The answer carries [`Extent::capped`], so that a caller can tell an end the file
    /// states from the cap without comparing the number to it — a stated end of exactly a
    /// megabyte is an end.
    ///
    /// **Asked at most once per symbol** (`ExtentCache`), which is what a section view
    /// scrolled away from and back does not pay for twice.
    pub fn extent(&self, object: &Object) -> Option<Extent> {
        *self.extent.0.get_or_init(|| {
            let extent = self.stated_extent(object)?;
            self.address.checked_add(extent.bytes).map(|_| extent)
        })
    }

    /// The three answers [`extent`](Self::extent) chooses among, before it bounds them.
    ///
    /// A length the file states is clamped to the next symbol. A listing is one stretch per
    /// symbol and decodes each as its symbol's extent, so a length reaching past the next
    /// label would draw those rows twice.
    fn stated_extent(&self, object: &Object) -> Option<Extent> {
        let derived = self.derived(object);
        let clamp = |bytes: u64| Extent {
            bytes: derived.map_or(bytes, |derived| bytes.min(derived)),
            capped: false,
        };
        if let Some(bytes) = self.unwind_extent() {
            return Some(clamp(bytes));
        }
        if let Some(bytes) = self.declared_extent(object.format) {
            return Some(clamp(bytes));
        }
        let estimate = derived.map(cap);
        let stated = |bytes| Extent {
            bytes,
            capped: false,
        };
        match (self.debug_extent(object).map(stated), estimate) {
            (Some(declared), Some(estimate)) if estimate.bytes < declared.bytes => Some(estimate),
            (declared, estimate) => declared.or(estimate),
        }
    }
}

/// The next-symbol derivation as an estimate: cut at [`MAX_DERIVED_SIZE`], and saying so.
/// The only place the cap is applied.
fn cap(derived: u64) -> Extent {
    Extent {
        bytes: derived.min(MAX_DERIVED_SIZE),
        capped: derived > MAX_DERIVED_SIZE,
    }
}

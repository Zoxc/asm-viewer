mod demangle;
mod disasm;
mod extent;
pub mod guard;
mod line;
mod listing;
mod made_up;
mod model;
mod open;
mod parse;
mod unwind;

pub use disasm::{Assembly, BranchEdge, Instruction, Operand, SpanKind};
pub use extent::{Extent, ExtentCache};
pub use line::{DebugInfoCache, LineInfo, LineRow, Location, SourceDigests, SourceHash};
pub use listing::{CodeListing, DecodedStretch, Gap, GapKind, Listing, Place, Placed, Stretch};
pub use made_up::MadeUp;
pub use model::{FileDigest, Object, ObjectData, PlacedSymbols, Section, Symbol, SymbolData};
pub use open::{open_data_streaming, open_files, open_files_streaming, Progress};
pub use parse::parse_object;
// Re-exported so the viewer needs no `object` dependency of its own.
pub use object::{Architecture, BinaryFormat, SectionIndex, SymbolIndex};

/// [`Object`] is shared as an `Arc` and read from worker threads; the others are what a
/// worker is handed and hands back. Asserted here so a field that stops being shared-safe
/// is a compile error in this crate rather than a borrow error in the UI.
const _: fn() = || {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<Object>();
    assert_send_sync::<Symbol>();
    assert_send_sync::<Assembly>();
    assert_send_sync::<LineInfo>();
    assert_send_sync::<Listing>();
    assert_send_sync::<CodeListing>();
    assert_send_sync::<DecodedStretch>();
};

//! The diff model behind Quark's diff view, with no UI dependencies.
//!
//! - [`parse_unified`] reads unified diffs (git's format with renames,
//!   copies, modes, and binary markers, or plain `diff -u`), and
//!   [`write_unified`] writes git's format back. [`apply`] applies a file
//!   diff to its old text.
//! - [`diff_texts`] diffs two whole texts by line (Myers' algorithm, see
//!   [`myers`]), keeping both texts so collapsed context can be expanded.
//! - [`inline_diff`] finds the changed words of a pair of changed lines.
//! - [`DiffDocument`] stores files, hunks, and blocks as flat column
//!   tables; [`Projection`] turns it into display rows, unified or side by
//!   side, with gaps of unchanged lines collapsed per an [`Expansion`] and
//!   its [`ContextPolicy`].
//! - [`Comparison`] aligns changed lines for display (similarity pairing,
//!   whitespace policies) without touching the exact document;
//!   [`paired_inline_diff`] finds changed words or graphemes of a pair.
//! - [`DiffDocument::hydrate_file`] swaps a patch's fragments for checked
//!   whole sources.
//! - [`DiffSession`] holds files of a changing diff by stable [`FileId`] and
//!   monotonic [`Revision`]; [`SourceRemap`] carries source lines across
//!   revisions.
//! - [`DiffLimits`] refuses input the model cannot represent and bounds
//!   per-line detail.
//!
//! Coordinates: source lines are zero-based, byte offsets are UTF-8 bytes,
//! ranges are half-open; display line numbers are one-based.

mod compare;
mod compute;
#[doc(hidden)]
pub mod fixtures;
mod inline;
mod limits;
mod model;
pub mod myers;
mod patch;
mod projection;
mod remap;
mod session;
mod source;
mod text;

#[cfg(test)]
mod tests;

pub use compare::{
    Comparison, ComparisonError, ComparisonOptions, LinePair, MAX_PAIRING_COMPARISONS,
    PAIRING_COMPARISONS_PER_LINE, PairingMode, SIMILARITY_SCAN_BYTES, WhitespaceMode,
};
pub use compute::{DEFAULT_CONTEXT, diff_texts};
pub use inline::{
    InlineDetail, InlineDiff, InlineMode, InlineOptions, MAX_INLINE_LINE_BYTES, MIN_KEPT_SHARE,
    PairedInlineDiff, inline_diff, paired_inline_diff,
};
pub use limits::{
    DETAIL_SIDE_BYTES, DiffError, DiffLimits, LimitKind, LineDetail, MAX_REPRESENTABLE,
    diff_texts_checked, line_detail, parse_unified_checked,
};
pub use model::{
    BlockKind, BlockTable, DiffDocument, FileFacts, FileMeta, FileStatus, FileSummary, FileTable,
    HunkTable, IntegrityError, Side,
};
pub use patch::{ApplyError, PatchError, apply, parse_unified, write_unified};
pub use projection::{
    ContextPolicy, Expansion, GapId, GapRow, MIN_HIDDEN, Mode, NONE, Projection, ProjectionError,
    REVEAL_STEP, Reveal, RowKind,
};
pub use remap::{LineMap, RemapError, SourceRemap};
pub use session::{
    DiffSession, DiffUpdate, FileDiffSnapshot, FileId, Revision, SessionIntegrityError,
    SourcePurpose, SourceRequest, UpdateError, UpdateOutcome,
};
pub use source::{ContextLen, FileSources, HydrationError, SourceCoverage};
pub use text::TextStore;

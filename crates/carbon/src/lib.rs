//! Carbon is Diffy's data-oriented diff substrate.
//!
//! The crate deliberately avoids UI, Git, text shaping, renderer, and syntax
//! dependencies. It owns durable diff coordinates and projections that the app
//! can adapt into native rendering structures.

pub mod inline;
pub mod model;
pub mod patch;
pub mod projection;
pub mod review;
pub mod text;

pub use inline::{
    ChangeIntensity, InlineDiff, InlineDiffMode, InlineOptions, InlineSpan, compute_inline_diff,
};
pub use model::{
    Block, BlockId, BlockKind, BlockRange, DiffDocument, DiffSide, FileDiff, FileId, FileMode,
    FileStatus, Hunk, HunkId, ObjectId, SourceRange,
};
pub use patch::{PatchError, parse_unified_patch};
pub use projection::{
    ExpansionDirection, ExpansionState, HunkExpansion, ProjectionBuffer, ProjectionMode,
    ProjectionOptions, ProjectionRow, ProjectionRowKind, ProjectionWindow, expand_context,
    map_anchor_to_projection, project_file, project_window, projected_row_byte_range,
};
pub use review::{
    Anchor, Annotation, AnnotationId, AnnotationKind, AnnotationSet, ByteRange, ConflictResolution,
    DiagnosticSeverity, LineRange, SuggestedChange,
};
pub use text::{
    LineId, TextByteRange, TextStore, u32_to_usize_saturating, usize_to_u32_saturating,
};

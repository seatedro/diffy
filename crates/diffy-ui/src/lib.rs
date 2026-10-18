//! Diffy's components built on Quark.

pub mod diff_view;

pub use diff_view::{
    AnnotationId, CopyContent, CopySide, DiffAnchor, DiffAnnotation, DiffEvent, DiffKey,
    DiffOutcome, DiffPreviewLimit, DiffSessionViewState, DiffStyle, DiffTarget, DiffViewState,
    FindOptions, RevealAlign, SearchCoverage, SearchDirection, SearchSides, SearchSummary,
    SourcePoint, diff_session_view, diff_session_view_with, diff_view, diff_view_with,
};

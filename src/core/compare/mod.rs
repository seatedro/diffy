pub mod backends;
pub mod progress;
pub mod service;
pub mod spec;
pub mod stats;
pub mod text;

pub use progress::{ComparePhase, ProgressSink};
pub use service::{CompareOutput, CompareService};
pub use spec::{CompareMode, CompareSpec, LayoutMode, RendererKind};
pub use stats::{
    COMPARE_SUMMARY_FILE_LIMIT, CompareFileStatsTarget, CompareFileSummary, ComparePath,
    u32_to_i32_saturating,
};
pub use text::compare_text;

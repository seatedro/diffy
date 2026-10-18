pub mod backends;
pub mod git;
pub mod output;
pub mod progress;
#[cfg(feature = "difftastic")]
pub mod semantic;
pub mod service;
pub mod spec;
pub mod stats;
pub mod text;

pub use output::{CompareEntry, CompareOutput};
pub use progress::{ComparePhase, ProgressSink};
pub use service::CompareService;
pub use spec::{CompareMode, CompareSpec, LayoutMode, RendererKind};
pub use stats::{COMPARE_SUMMARY_FILE_LIMIT, ComparePath, u32_to_i32_saturating};
pub use text::compare_text;

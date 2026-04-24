pub mod backends;
pub mod progress;
pub mod service;
pub mod spec;

pub use progress::{ComparePhase, ProgressSink};
pub use service::{CompareOutput, CompareService};
pub use spec::{CompareMode, CompareSpec, LayoutMode, RendererKind};

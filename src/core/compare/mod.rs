pub mod spec;
pub mod service;
pub mod backends;

pub use service::{CompareOutput, CompareService};
pub use spec::{CompareMode, CompareSpec, LayoutMode, RendererKind};

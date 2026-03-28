pub mod backends;
pub mod service;
pub mod spec;

pub use service::{CompareOutput, CompareService};
pub use spec::{CompareMode, CompareSpec, LayoutMode, RendererKind};

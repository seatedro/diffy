#[cfg(test)]
pub mod fixtures;
mod keys;
mod state;
mod syntax;
mod targets;
#[cfg(test)]
mod tests;
mod text_compare;
mod view;

pub use state::{InteractionController, Phase, SearchBar};
pub use syntax::SyntaxWake;
pub use targets::{hunk_target, source_target};
pub use text_compare::TextCompare;
pub use view::{view, view_shared};

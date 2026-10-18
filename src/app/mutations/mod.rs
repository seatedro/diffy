mod presentation;
mod state;
#[cfg(test)]
mod tests;
mod view;

pub use presentation::{MutationPresentation, PresentationStyle};
pub use state::{LineKey, MutationsController};
pub use view::{panel, view};

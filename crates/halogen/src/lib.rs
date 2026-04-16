//! Halogen — declarative UI toolkit with fine-grained reactivity.
//!
//! - `view!` declarative macro (re-exported from `halogen_macros`)
//! - `reactive` module with `Signal`, `SignalStore`, memos, tracking

pub use halogen_macros::{Store, view};

pub mod reactive;

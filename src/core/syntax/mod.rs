pub mod grammar;
pub mod highlighter;
#[cfg(test)]
mod phosphor_parity;
pub mod types;

pub use grammar::{GrammarConfig, Grammars};
pub use highlighter::Highlighter;
pub use types::SyntaxTokenKind;

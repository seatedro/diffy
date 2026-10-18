pub mod annotator;
pub mod highlighter;
#[cfg(test)]
mod phosphor_parity;
pub mod types;

pub use annotator::DiffSyntaxAnnotator;
pub use highlighter::Highlighter;
pub use types::SyntaxTokenKind;

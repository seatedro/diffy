pub mod annotator;
pub mod highlighter;
pub mod language_registry;
pub mod types;

pub use annotator::DiffSyntaxAnnotator;
pub use highlighter::Highlighter;
pub use language_registry::{Grammar, LanguageRegistry};
pub use types::SyntaxTokenKind;

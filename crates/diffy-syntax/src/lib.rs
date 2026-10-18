//! Tree-sitter syntax highlighting for code blocks, with grammars loaded at
//! runtime.
//!
//! No grammar is compiled in. Each language arrives as a pack, a shared
//! library exporting the tree-sitter language function plus its queries
//! and a manifest (see [`pack`]), from a [`GrammarStore`] the app
//! configures: local pack directories it trusts, and with the `download`
//! feature a signed index it downloads missing packs from. Until a
//! language's grammar is available its code renders plain.
//!
//! [`highlight`] runs synchronously; [`HighlightWorker`] runs it on a
//! background thread and drops requests superseded by a newer generation
//! for the same slot, which suits a code block that is still streaming.
//! Several views can share one worker thread through
//! [`HighlightWorker::share`]. Text made of excerpts, such as the hunks of
//! a patch, is highlighted one excerpt at a time ([`highlight_fragments`],
//! [`HighlightRequest::fragments`]) so lexical state does not leak from one
//! into the next.
//!
//! Languages embedded in others (a script in HTML, a fenced block in
//! Markdown, a macro body in Rust) are highlighted with their own grammars
//! when the host's pack has an injection query, following tree-sitter's
//! injection rules. Embedded spans take precedence over the host's. While
//! an embedded grammar downloads, its region keeps the host's colors, and
//! the worker sends a newer revision of the result when it arrives.
//!
//! Features: `engine` (the tree-sitter runtime and local packs) and
//! `download` (fetching packs). Without `engine` every lookup is plain and
//! the crate has no dependencies.
//!
//! # Threat model
//!
//! Loading a pack runs native code from it with the app's privileges, so
//! the question for every pack is who could have written its bytes.
//!
//! - Downloaded packs are trusted only through the index signature: the
//!   index must verify against an Ed25519 key the app compiled in, and
//!   every file must match the SHA-256 the signed index lists, both when
//!   it is downloaded and each time it is loaded from the cache. There is
//!   no switch to accept unsigned indexes. The index names its target
//!   triple, so a signed index for another platform is refused; each
//!   manifest's file names must be plain file names, its symbol a C
//!   identifier, its library this platform's extension, and its ABI one
//!   this runtime supports. A compromised host or network can withhold
//!   packs or serve an older signed index (a downgrade to packs that were
//!   once signed), but cannot get unsigned code loaded.
//! - Local packs are not signed. They load only from directories the app
//!   names ([`StoreConfig::local_packs`]); their manifests are checked the
//!   same way, and their SHA-256 sums catch corruption, not tampering.
//!   Name only directories that only the app's installer or the developer
//!   can write, never a downloads folder or a world-writable path.
//! - Anyone who can write the user's cache directory can already run code
//!   as the user, so the cache is not defended against local attackers
//!   beyond the per-load checks above. A library swapped between its
//!   SHA-256 check and `dlopen` is out of scope for the same reason.
//! - Grammars parse untrusted text (chat output, diffs). A parser crash
//!   takes the process down; a panic in the highlighter is caught by the
//!   worker. Packs come from pinned, hashed grammar sources built in CI.

#[cfg(feature = "download")]
mod download;
#[cfg(feature = "engine")]
mod engine;
#[cfg(feature = "engine")]
pub mod pack;
mod store;
#[cfg(feature = "engine")]
#[doc(hidden)]
pub mod testing;
mod worker;

use std::sync::Arc;

#[cfg(feature = "download")]
pub use download::{Downloads, default_cache_dir};
#[cfg(feature = "download")]
pub use quark_update::PublicKey;
#[cfg(feature = "engine")]
pub use store::StoreConfig;
pub use store::{GrammarStore, LanguageStatus};
pub use worker::{
    HighlightFocus, HighlightPart, HighlightRequest, HighlightWorker, Highlighted, Priority,
    WorkerGone,
};

/// What a highlighted run of source is.
#[repr(u8)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum HighlightKind {
    #[default]
    Normal = 0,
    Keyword,
    String,
    Comment,
    Number,
    Type,
    Function,
    Operator,
    Punctuation,
    Variable,
    Constant,
    Builtin,
    Attribute,
    Tag,
    Property,
    Namespace,
    Label,
    Preprocessor,
}

impl HighlightKind {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Keyword => "keyword",
            Self::String => "string",
            Self::Comment => "comment",
            Self::Number => "number",
            Self::Type => "type",
            Self::Function => "function",
            Self::Operator => "operator",
            Self::Punctuation => "punctuation",
            Self::Variable => "variable",
            Self::Constant => "constant",
            Self::Builtin => "builtin",
            Self::Attribute => "attribute",
            Self::Tag => "tag",
            Self::Property => "property",
            Self::Namespace => "namespace",
            Self::Label => "label",
            Self::Preprocessor => "preprocessor",
        }
    }
}

/// A highlighted byte range of the source. Spans are sorted and do not
/// overlap; bytes outside every span are [`HighlightKind::Normal`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HighlightSpan {
    pub offset: u32,
    pub length: u32,
    pub kind: HighlightKind,
}

impl HighlightSpan {
    pub fn range(self) -> std::ops::Range<usize> {
        // Saturates like `compact_spans` does for sources past 4 GiB.
        self.offset as usize..self.offset.saturating_add(self.length) as usize
    }
}

/// A language as a code block or file names it: a fence tag (```` ```rs ````)
/// or a file extension, lowercased. Which grammar it means, if any, is up to
/// the [`GrammarStore`]: packs list their aliases and extensions.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LanguageId(Arc<str>);

impl LanguageId {
    /// `None` for tags no pack could match: empty, longer than 32 bytes, or
    /// with characters other than ASCII letters, digits, and `+#._-`.
    pub fn from_fence(tag: &str) -> Option<Self> {
        let valid = !tag.is_empty()
            && tag.len() <= 32
            && tag.bytes().all(|b| {
                b.is_ascii_alphanumeric() || matches!(b, b'+' | b'#' | b'.' | b'_' | b'-')
            });
        valid.then(|| Self(Arc::from(tag.to_ascii_lowercase())))
    }

    /// The language a file path's extension names.
    pub fn from_path(path: &str) -> Option<Self> {
        let name = path.rsplit(['/', '\\']).next()?;
        let (_, extension) = name.rsplit_once('.')?;
        Self::from_fence(extension)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Whether `tag` names this language, without allocating; for callers
    /// that check a tag on every rebuild.
    pub fn matches(&self, tag: &str) -> bool {
        self.0.eq_ignore_ascii_case(tag)
    }
}

impl std::fmt::Display for LanguageId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Highlights `source` on the calling thread, including the languages
/// embedded in it. Empty when `store` has no grammar for the language yet;
/// highlighting is best effort. Resolving a language for the first time
/// may load its pack from disk or start its download.
pub fn highlight(store: &GrammarStore, language: &LanguageId, source: &str) -> Vec<HighlightSpan> {
    store.highlight(language, source).spans
}

/// [`highlight`] over each of `fragments` (sorted, disjoint byte ranges of
/// `source`, on character boundaries) as a document of its own: a string or
/// comment left open at the end of one does not continue into the next.
/// Spans are in `source`'s coordinates and lie inside the fragments.
pub fn highlight_fragments(
    store: &GrammarStore,
    language: &LanguageId,
    source: &str,
    fragments: &[std::ops::Range<u32>],
) -> Vec<HighlightSpan> {
    store
        .highlight_fragments_until(language, source, fragments, &|| false)
        .spans
}

#[cfg(all(test, feature = "download"))]
mod tests;

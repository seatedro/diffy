//! Grammars compiled into the binary with the `bundled` feature.
//!
//! `syntax-pack bundle` compiles a fixed set of grammars from their pinned
//! sources into static libraries and writes their queries and a table;
//! `build.rs` links them when `DIFFY_SYNTAX_BUNDLE` names that directory.
//! Without it the bundle is empty. A store uses them only after
//! [`crate::StoreConfig::bundled`].

/// One compiled-in grammar, as `build.rs` generates it.
pub(crate) struct Bundled {
    pub(crate) name: &'static str,
    pub(crate) aliases: &'static [&'static str],
    pub(crate) extensions: &'static [&'static str],
    /// The grammar's tree-sitter ABI, checked against the linked grammar.
    pub(crate) abi: u32,
    pub(crate) language: unsafe extern "C" fn() -> *const (),
    /// SHA-256 of the pinned sources it was built from.
    pub(crate) source: &'static str,
    pub(crate) highlights: &'static str,
    pub(crate) injections: Option<&'static str>,
    pub(crate) locals: Option<&'static str>,
    pub(crate) semantic: Option<&'static str>,
    pub(crate) tags: Option<&'static str>,
}

impl Bundled {
    /// The canonical name, aliases, and extensions, each a lookup key.
    pub(crate) fn tags(&self) -> impl Iterator<Item = &'static str> {
        std::iter::once(self.name)
            .chain(self.aliases.iter().copied())
            .chain(self.extensions.iter().copied())
    }
}

include!(concat!(env!("OUT_DIR"), "/bundled.rs"));

/// The canonical names of the grammars compiled into this build; empty
/// when it was built without `DIFFY_SYNTAX_BUNDLE`.
pub fn languages() -> impl ExactSizeIterator<Item = &'static str> + Clone {
    GRAMMARS.iter().map(|grammar| grammar.name)
}

/// The bundled grammar a name, alias, or extension names.
pub(crate) fn find(tag: &str) -> Option<&'static Bundled> {
    GRAMMARS
        .iter()
        .find(|grammar| grammar.tags().any(|known| known.eq_ignore_ascii_case(tag)))
}

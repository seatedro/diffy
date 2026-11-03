//! Grammars compiled in with the `bundled` feature, from the
//! `syntax-pack bundle` output `DIFFY_SYNTAX_BUNDLE` names at build time
//! (empty without it). Stores use them only after
//! [`crate::StoreConfig::bundled`].

/// One compiled-in grammar, as `build.rs` generates it.
pub(crate) struct Bundled {
    pub(crate) name: &'static str,
    pub(crate) aliases: &'static [&'static str],
    pub(crate) extensions: &'static [&'static str],
    /// Declared tree-sitter ABI, checked against the linked grammar.
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
    /// Lookup keys: name, aliases, extensions.
    pub(crate) fn tags(&self) -> impl Iterator<Item = &'static str> {
        std::iter::once(self.name)
            .chain(self.aliases.iter().copied())
            .chain(self.extensions.iter().copied())
    }
}

include!(concat!(env!("OUT_DIR"), "/bundled.rs"));

/// Names of the compiled-in grammars; empty without `DIFFY_SYNTAX_BUNDLE`.
pub fn languages() -> impl ExactSizeIterator<Item = &'static str> + Clone {
    GRAMMARS.iter().map(|grammar| grammar.name)
}

/// By name, alias, or extension, ignoring ASCII case.
pub(crate) fn find(tag: &str) -> Option<&'static Bundled> {
    GRAMMARS
        .iter()
        .find(|grammar| grammar.tags().any(|known| known.eq_ignore_ascii_case(tag)))
}

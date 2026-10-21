//! Test support for crates whose tests highlight real code.
//!
//! Tests load packs from the root `syntax-pack build` writes by default
//! (`<workspace>/target/syntax-packs`), or from `$DIFFY_SYNTAX_TEST_PACKS`.
//! Build the packs a test names once with `cargo run -p syntax-pack --
//! build <language>...`. Without them these tests skip, unless
//! `DIFFY_REQUIRE_SYNTAX_PACKS=1` (set in CI after building the fixtures)
//! turns a missing pack into a failure.

use std::path::PathBuf;

use crate::{GrammarStore, StoreConfig, pack};

/// The pack root tests read.
pub fn pack_root() -> PathBuf {
    std::env::var_os("DIFFY_SYNTAX_TEST_PACKS")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/syntax-packs")
        })
}

/// A store over [`pack_root`] when it holds a pack for `language`.
pub fn store_with(language: &str) -> Option<GrammarStore> {
    store_with_all(&[language])
}

/// A store over [`pack_root`] when it holds a pack for every one of
/// `languages`.
pub fn store_with_all(languages: &[&str]) -> Option<GrammarStore> {
    let root = pack_root();
    for language in languages {
        let dir = root.join(pack::TARGET).join(language);
        if dir.join(pack::MANIFEST).is_file() {
            continue;
        }
        assert!(
            std::env::var_os("DIFFY_REQUIRE_SYNTAX_PACKS").is_none_or(|v| v != "1"),
            "DIFFY_REQUIRE_SYNTAX_PACKS=1 but there is no {language} pack in {}; \
             build it with `cargo run -p syntax-pack -- build {language}`",
            dir.display()
        );
        eprintln!(
            "skipping: no {language} pack in {}; build it with `cargo run -p syntax-pack -- build {language}`",
            dir.display()
        );
        return None;
    }
    Some(GrammarStore::new(StoreConfig::new().local_packs(root)))
}

pub fn store_deferring(languages: &[&str], deferred: &[&str]) -> Option<GrammarStore> {
    store_with_all(languages)?;
    Some(GrammarStore::new(
        StoreConfig::new().local_packs(pack_root()).defer(deferred),
    ))
}

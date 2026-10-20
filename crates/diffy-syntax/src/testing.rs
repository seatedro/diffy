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

#[cfg(feature = "download")]
pub mod legacy {
    use std::path::{Path, PathBuf};

    use serde_json::{Value, json};

    use crate::legacy_pack::{self, LegacyPacks, MANIFEST, platform};
    use crate::{GrammarStore, PublicKey, StoreConfig, pack};

    pub const SEED: [u8; 32] = [11; 32];

    pub fn key() -> Option<PublicKey> {
        quark_update::manifest::public_key(&SEED).ok()
    }

    pub fn sign(payload: &Value) -> Option<String> {
        quark_update::manifest::sign(&SEED, payload).ok()
    }

    pub fn sha(bytes: &[u8]) -> String {
        pack::hex(ring::digest::digest(&ring::digest::SHA256, bytes).as_ref())
    }

    pub fn parser_name() -> String {
        format!("parser{}", legacy_pack::library_suffix(platform()))
    }

    #[derive(Debug, Clone)]
    pub struct PublishedPack {
        pub language: String,
        pub version: String,
        pub symbol: String,
        pub parser: Vec<u8>,
        pub highlights: Vec<u8>,
        pub injections: Option<Vec<u8>>,
    }

    impl PublishedPack {
        pub fn new(language: &str, parser: &[u8], highlights: &[u8]) -> Self {
            Self {
                language: language.to_owned(),
                version: "a1b2c3d4e5f6-0a1b2c3d4e5f".to_owned(),
                symbol: format!("tree_sitter_{language}"),
                parser: parser.to_vec(),
                highlights: highlights.to_vec(),
                injections: None,
            }
        }

        pub fn from_native(language: &str, native: &str) -> Option<Self> {
            let dir = super::pack_root().join(pack::TARGET).join(native);
            let manifest = pack::read_manifest(&dir).ok()?;
            let read = |name: &str| std::fs::read(dir.join(name)).ok();
            Some(Self {
                language: language.to_owned(),
                version: format!("{}-{}", manifest.version.replace('.', ""), native),
                symbol: manifest.symbol.clone(),
                parser: read(&manifest.library.path)?,
                highlights: read(&manifest.highlights.path)?,
                injections: match &manifest.injections {
                    Some(file) => Some(read(&file.path)?),
                    None => None,
                },
            })
        }

        pub fn manifest(&self) -> Vec<u8> {
            let manifest = json!({
                "schema_version": legacy_pack::SCHEMA,
                "language": self.language,
                "version": self.version,
                "platform": platform(),
                "tree_sitter_abi": legacy_pack::expected_abi(),
                "symbol": self.symbol,
                "parser": { "path": parser_name(), "sha256": sha(&self.parser) },
                "highlights": { "path": "highlights.scm", "sha256": sha(&self.highlights) },
                "injections": self.injections.as_ref().map(|bytes| {
                    json!({ "path": "injections.scm", "sha256": sha(bytes) })
                }),
                "extensions": [self.language],
                "source": {},
            });
            let mut bytes = serde_json::to_vec_pretty(&manifest).unwrap_or_default();
            bytes.push(b'\n');
            bytes
        }

        pub fn files(&self) -> Vec<(String, Vec<u8>)> {
            let mut files = vec![
                (parser_name(), self.parser.clone()),
                ("highlights.scm".to_owned(), self.highlights.clone()),
            ];
            if let Some(injections) = &self.injections {
                files.push(("injections.scm".to_owned(), injections.clone()));
            }
            files.push((MANIFEST.to_owned(), self.manifest()));
            files
        }

        pub fn url_path(&self, file: &str) -> String {
            format!("/packs/{}/{}/{}", self.language, self.version, file)
        }

        pub fn entry(&self, base_url: &str) -> Value {
            let remote = |path: &str, bytes: &[u8]| {
                json!({
                    "url": format!("{base_url}{}", self.url_path(path)),
                    "path": path,
                    "sha256": sha(bytes),
                })
            };
            json!({
                "language": self.language,
                "version": self.version,
                "common": true,
                "extensions": [self.language],
                "symbol": self.symbol,
                "manifest": remote(MANIFEST, &self.manifest()),
                "parser": remote(&parser_name(), &self.parser),
                "highlights": remote("highlights.scm", &self.highlights),
                "injections": self.injections.as_ref().map(|bytes| remote("injections.scm", bytes)),
                "source": {},
            })
        }

        pub fn dir(&self, storage: &Path) -> PathBuf {
            storage
                .join(&self.language)
                .join(&self.version)
                .join(platform())
        }

        pub fn install(&self, storage: &Path) -> std::io::Result<PathBuf> {
            let dir = self.dir(storage);
            std::fs::create_dir_all(&dir)?;
            for (name, bytes) in self.files() {
                std::fs::write(dir.join(name), bytes)?;
            }
            Ok(dir)
        }
    }

    pub fn index(entries: Vec<Value>) -> Value {
        json!({
            "schema_version": legacy_pack::SCHEMA,
            "generated_from": "fixture",
            "generated_at": "2026-01-01T00:00:00Z",
            "platform": platform(),
            "tree_sitter_abi": legacy_pack::expected_abi(),
            "packs": entries,
        })
    }

    pub fn write_index(storage: &Path, signed: &str) -> std::io::Result<()> {
        let dir = storage.join(".index");
        std::fs::create_dir_all(&dir)?;
        std::fs::write(dir.join(format!("{}.json", platform())), signed)
    }

    pub fn install_signed(storage: &Path, packs: &[PublishedPack]) -> Option<()> {
        for pack in packs {
            pack.install(storage).ok()?;
        }
        let entries = packs
            .iter()
            .map(|pack| pack.entry("http://127.0.0.1:9"))
            .collect();
        write_index(storage, &sign(&index(entries))?).ok()
    }

    pub fn native_packs(languages: &[(&str, &str)]) -> Option<Vec<PublishedPack>> {
        let mut packs = Vec::new();
        for (language, native) in languages {
            super::store_with(native)?;
            packs.push(PublishedPack::from_native(language, native)?);
        }
        Some(packs)
    }

    pub fn cached_store(storage: &Path) -> Option<GrammarStore> {
        Some(GrammarStore::new(
            StoreConfig::new().legacy_packs(LegacyPacks::new(storage, &[key()?])),
        ))
    }
}

/// [`store_with_all`], except that the packs of `deferred` stay pending,
/// as if downloading, until [`GrammarStore::release_deferred`].
pub fn store_deferring(languages: &[&str], deferred: &[&str]) -> Option<GrammarStore> {
    store_with_all(languages)?;
    Some(GrammarStore::new(
        StoreConfig::new().local_packs(pack_root()).defer(deferred),
    ))
}

//! The grammar pack format and its checks.
//!
//! A pack is one language for one target triple: a directory holding a
//! shared library that exports the tree-sitter language function, the
//! highlight query, an optional injection query, and `manifest.json`:
//!
//! ```json
//! {
//!   "schema": 1,
//!   "language": "rust",
//!   "aliases": ["rs"],
//!   "extensions": ["rs"],
//!   "version": "0.24.2",
//!   "target": "x86_64-unknown-linux-gnu",
//!   "abi": 15,
//!   "symbol": "tree_sitter_rust",
//!   "library": { "path": "libtree-sitter-rust.so", "sha256": "…", "size": 1120344 },
//!   "highlights": { "path": "highlights.scm", "sha256": "…", "size": 3211 },
//!   "injections": { "path": "injections.scm", "sha256": "…", "size": 310 },
//!   "source": { "repo": "https://github.com/tree-sitter/tree-sitter-rust", "rev": "e2bee85…", "sha256": "…" }
//! }
//! ```
//!
//! A pack root holds packs as `<root>/<target>/<language>/manifest.json`,
//! which is what `syntax-pack build` writes. A signed index
//! (`<root>/<target>/index.json`, see [`verify_index`]) lists the manifests
//! of every pack for one target; a file's URL is its `url` field or, when
//! that is absent, `<language>/<path>` relative to the index URL.
//!
//! Nothing in a manifest is trusted until it passes [`PackManifest::validate`]:
//! file names are single path segments, the library has this platform's
//! extension, the symbol is a C identifier, and the ABI is one this
//! tree-sitter runtime can load. Library bytes are checked against the
//! manifest's SHA-256 before they are loaded.

use std::fs::File;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use ring::digest::{Context, SHA256};
use serde::{Deserialize, Serialize};

/// The pack format this build reads and writes.
pub const SCHEMA: u32 = 1;

/// The target triple this build loads packs for, such as
/// `aarch64-apple-darwin`.
pub const TARGET: &str = env!("DIFFY_SYNTAX_TARGET");

/// File name of a pack's manifest.
pub const MANIFEST: &str = "manifest.json";

/// Grammar ABI versions (tree-sitter `LANGUAGE_VERSION`) this runtime can
/// load.
pub fn supported_abi() -> std::ops::RangeInclusive<u32> {
    tree_sitter::MIN_COMPATIBLE_LANGUAGE_VERSION as u32..=tree_sitter::LANGUAGE_VERSION as u32
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackManifest {
    pub schema: u32,
    /// Canonical name, also a fence tag: `rust`.
    pub language: String,
    /// Other fence tags: `rs`.
    #[serde(default)]
    pub aliases: Vec<String>,
    /// File extensions without the dot.
    #[serde(default)]
    pub extensions: Vec<String>,
    pub version: String,
    pub target: String,
    /// The grammar's tree-sitter ABI (`LANGUAGE_VERSION` in its parser.c).
    pub abi: u32,
    /// The exported language function.
    pub symbol: String,
    pub library: PackFile,
    pub highlights: PackFile,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub injections: Option<PackFile>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<PackSource>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackFile {
    /// A file name in the pack directory.
    pub path: String,
    /// Lowercase hex SHA-256 of the file.
    pub sha256: String,
    pub size: u64,
    /// Where an index serves the file, when not next to the index.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

/// The pinned grammar source a pack was built from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackSource {
    pub repo: String,
    pub rev: String,
    /// SHA-256 over the source files compiled and copied; see the
    /// `syntax-pack` tool.
    pub sha256: String,
}

/// The signed payload of an index: every pack for one target.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackIndex {
    pub schema: u32,
    pub target: String,
    pub packs: Vec<PackManifest>,
}

#[derive(Debug, thiserror::Error)]
pub enum PackError {
    #[error("pack I/O failed: {0}")]
    Io(#[from] io::Error),
    #[error("pack metadata is not valid JSON of the expected shape: {0}")]
    Malformed(String),
    #[error("pack schema {0} is not supported")]
    Schema(u32),
    #[error("no trusted index keys are configured")]
    NoTrustedKeys,
    #[error("index signature does not verify against any trusted key")]
    BadSignature,
    #[error("pack is for {found}, this build loads {expected}")]
    WrongTarget { expected: String, found: String },
    #[error("grammar ABI {abi} is outside the supported {min}..={max}")]
    Abi { abi: u32, min: u32, max: u32 },
    #[error("unsafe pack metadata in {field}: {value:?}")]
    Unsafe { field: &'static str, value: String },
    #[error("{path} does not match its SHA-256")]
    ShaMismatch { path: PathBuf },
    #[error("loading the grammar library failed: {0}")]
    Library(String),
    #[error("the highlight query does not compile: {0}")]
    Query(String),
    #[error("the injection query does not compile: {0}")]
    InjectionQuery(String),
}

impl PackManifest {
    /// Checks every field the loader acts on, for packs built for
    /// `target`. Run before a manifest's paths or symbol are used.
    pub fn validate(&self, target: &str) -> Result<(), PackError> {
        if self.schema != SCHEMA {
            return Err(PackError::Schema(self.schema));
        }
        if self.target != target {
            return Err(PackError::WrongTarget {
                expected: target.to_owned(),
                found: self.target.clone(),
            });
        }
        let abi = supported_abi();
        if !abi.contains(&self.abi) {
            return Err(PackError::Abi {
                abi: self.abi,
                min: *abi.start(),
                max: *abi.end(),
            });
        }
        check_tag("language", &self.language)?;
        for tag in self.aliases.iter().chain(&self.extensions) {
            check_tag("aliases", tag)?;
        }
        check_segment("version", &self.version)?;
        if !is_c_identifier(&self.symbol) {
            return Err(unsafe_field("symbol", &self.symbol));
        }
        check_file("library", &self.library)?;
        // A library with another platform's extension, or a query file
        // posing as a library, never reaches dlopen.
        if !self.library.path.ends_with(library_suffix(target)) {
            return Err(unsafe_field("library.path", &self.library.path));
        }
        check_file("highlights", &self.highlights)?;
        if let Some(injections) = &self.injections {
            check_file("injections", injections)?;
        }
        Ok(())
    }

    /// The canonical name, aliases, and extensions, each a lookup key.
    pub fn tags(&self) -> impl Iterator<Item = &str> {
        std::iter::once(self.language.as_str())
            .chain(self.aliases.iter().map(String::as_str))
            .chain(self.extensions.iter().map(String::as_str))
    }

    /// The files of the pack: library, highlights, and injections.
    pub fn files(&self) -> impl Iterator<Item = &PackFile> {
        [&self.library, &self.highlights]
            .into_iter()
            .chain(self.injections.as_ref())
    }
}

/// The shared library extension of a target triple.
pub fn library_suffix(target: &str) -> &'static str {
    if target.contains("windows") {
        ".dll"
    } else if target.contains("apple") {
        ".dylib"
    } else {
        ".so"
    }
}

pub(crate) fn unsafe_field(field: &'static str, value: &str) -> PackError {
    PackError::Unsafe {
        field,
        value: value.to_owned(),
    }
}

fn check_file(field: &'static str, file: &PackFile) -> Result<(), PackError> {
    check_segment(field, &file.path)?;
    if file.sha256.len() != 64 || !file.sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(unsafe_field(field, &file.sha256));
    }
    Ok(())
}

/// One ordinary file name: no separators, no `.` or `..`, no drive or
/// stream prefix, nothing a shell or Windows would treat specially.
pub(crate) fn check_segment(field: &'static str, value: &str) -> Result<(), PackError> {
    let ok = !value.is_empty()
        && value.len() <= 128
        && value != "."
        && value != ".."
        && !value.starts_with('.')
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_' | b'+'));
    if ok {
        Ok(())
    } else {
        Err(unsafe_field(field, value))
    }
}

/// Fence tags and extensions: short, lowercase-insensitive ASCII, and safe
/// as a directory name since the language name becomes one.
fn check_tag(field: &'static str, value: &str) -> Result<(), PackError> {
    check_segment(field, value)?;
    if value.len() > 32 {
        return Err(unsafe_field(field, value));
    }
    Ok(())
}

pub(crate) fn is_c_identifier(value: &str) -> bool {
    let mut bytes = value.bytes();
    value.len() <= 128
        && bytes
            .next()
            .is_some_and(|b| b.is_ascii_alphabetic() || b == b'_')
        && bytes.all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

/// Lowercase hex SHA-256 of a file.
pub fn sha256_file(path: &Path) -> io::Result<String> {
    let mut file = File::open(path)?;
    let mut context = Context::new(&SHA256);
    let mut buf = vec![0; 64 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        context.update(&buf[..n]);
    }
    Ok(hex(context.finish().as_ref()))
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        out.push(DIGITS[usize::from(byte >> 4)] as char);
        out.push(DIGITS[usize::from(byte & 0xf)] as char);
    }
    out
}

/// Checks each file of `manifest` in `dir` against its SHA-256.
pub fn verify_files(dir: &Path, manifest: &PackManifest) -> Result<(), PackError> {
    for file in manifest.files() {
        let path = dir.join(&file.path);
        if sha256_file(&path)? != file.sha256.to_ascii_lowercase() {
            return Err(PackError::ShaMismatch { path });
        }
    }
    Ok(())
}

/// Reads and validates `<dir>/manifest.json` for this build's target.
pub fn read_manifest(dir: &Path) -> Result<PackManifest, PackError> {
    let bytes = std::fs::read(dir.join(MANIFEST))?;
    let manifest: PackManifest =
        serde_json::from_slice(&bytes).map_err(|e| PackError::Malformed(e.to_string()))?;
    manifest.validate(TARGET)?;
    Ok(manifest)
}

/// Loads the pack in `dir` and compiles its highlight and injection
/// queries, as the store does before first use. For build tooling that smoke-tests packs.
pub fn check_pack_dir(dir: &Path) -> Result<PackManifest, PackError> {
    let manifest = read_manifest(dir)?;
    crate::engine::Grammar::load(dir, &manifest)?;
    Ok(manifest)
}

#[cfg(feature = "download")]
pub(crate) use signed::unhex;
#[cfg(feature = "download")]
pub use signed::{VerifiedIndex, verify_index};

#[cfg(feature = "download")]
mod signed {
    use ring::signature::{ED25519, UnparsedPublicKey};
    use serde::Deserialize;
    use serde_json::Value;

    use super::{PackError, PackIndex, PackManifest, SCHEMA};

    /// An index whose signature verified, with the packs this build can use
    /// and the ones it cannot (wrong ABI, unsafe metadata) and why.
    #[derive(Debug)]
    pub struct VerifiedIndex {
        pub packs: Vec<PackManifest>,
        pub rejected: Vec<(String, PackError)>,
    }

    #[derive(Deserialize)]
    struct Envelope {
        payload: Value,
        signature: String,
    }

    /// Parses a signed index (the envelope `quark_update::manifest::sign`
    /// writes: `{"payload": …, "signature": "<hex>"}`), checks the Ed25519
    /// signature over the payload's canonical JSON against `keys`, and
    /// then that it is for `target`. Nothing in the payload is read before
    /// the signature verifies. Each pack is validated on its own, so one
    /// bad entry costs only its language.
    pub fn verify_index(
        bytes: &[u8],
        keys: &[[u8; 32]],
        target: &str,
    ) -> Result<VerifiedIndex, PackError> {
        let payload = verify_signed(bytes, keys)?;
        let index: PackIndex =
            serde_json::from_value(payload).map_err(|e| PackError::Malformed(e.to_string()))?;
        if index.schema != SCHEMA {
            return Err(PackError::Schema(index.schema));
        }
        if index.target != target {
            return Err(PackError::WrongTarget {
                expected: target.to_owned(),
                found: index.target,
            });
        }
        let mut verified = VerifiedIndex {
            packs: Vec::new(),
            rejected: Vec::new(),
        };
        for pack in index.packs {
            match pack.validate(target) {
                Ok(()) => verified.packs.push(pack),
                Err(error) => verified.rejected.push((pack.language, error)),
            }
        }
        Ok(verified)
    }

    pub(crate) fn verify_signed(bytes: &[u8], keys: &[[u8; 32]]) -> Result<Value, PackError> {
        if keys.is_empty() {
            return Err(PackError::NoTrustedKeys);
        }
        let envelope: Envelope =
            serde_json::from_slice(bytes).map_err(|e| PackError::Malformed(e.to_string()))?;
        let signature = unhex(&envelope.signature).ok_or(PackError::BadSignature)?;
        let message = canonical_json(&envelope.payload);
        let trusted = keys.iter().any(|key| {
            UnparsedPublicKey::new(&ED25519, key)
                .verify(message.as_bytes(), &signature)
                .is_ok()
        });
        if !trusted {
            return Err(PackError::BadSignature);
        }
        Ok(envelope.payload)
    }

    pub(crate) fn unhex(text: &str) -> Option<Vec<u8>> {
        let text = text.trim().as_bytes();
        if !text.len().is_multiple_of(2) {
            return None;
        }
        let nibble = |c: u8| (c as char).to_digit(16).map(|d| d as u8);
        text.as_chunks::<2>()
            .0
            .iter()
            .map(|&[hi, lo]| Some((nibble(hi)? << 4) | nibble(lo)?))
            .collect()
    }

    /// The same canonical form quark-update signs: object keys sorted, no
    /// whitespace, scalars as compact serde_json prints them.
    fn canonical_json(value: &Value) -> String {
        let mut out = String::new();
        write_canonical(value, &mut out);
        out
    }

    fn write_canonical(value: &Value, out: &mut String) {
        match value {
            Value::Array(items) => {
                out.push('[');
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    write_canonical(item, out);
                }
                out.push(']');
            }
            Value::Object(map) => {
                let mut keys: Vec<&String> = map.keys().collect();
                keys.sort();
                out.push('{');
                for (i, key) in keys.into_iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    out.push_str(&Value::String(key.clone()).to_string());
                    out.push(':');
                    write_canonical(&map[key], out);
                }
                out.push('}');
            }
            scalar => out.push_str(&scalar.to_string()),
        }
    }
}

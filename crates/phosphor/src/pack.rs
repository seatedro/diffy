use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use libloading::Library;
use ring::signature::{ED25519, UnparsedPublicKey};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tree_sitter as ts;
use tree_sitter_language::LanguageFn;

use crate::LanguageId;

const DEFAULT_INDEX_BASE_URL: &str = "https://blob.diffygui.com/phosphor-index";

const INDEX_PUBLIC_KEY_ENV: &str = "PHOSPHOR_PACK_INDEX_PUBLIC_KEY";
const ALLOW_UNSIGNED_ENV: &str = "DIFFY_PHOSPHOR_ALLOW_UNSIGNED_PACKS";

pub type Result<T> = std::result::Result<T, PackError>;

#[derive(Debug, thiserror::Error)]
pub enum PackError {
    #[error("syntax pack storage directory is unavailable")]
    MissingStorageDir,
    #[error(
        "syntax pack index has no trusted public key; set {INDEX_PUBLIC_KEY_ENV} at build time"
    )]
    MissingTrustedKey,
    #[error("syntax pack index signature is invalid")]
    InvalidSignature,
    #[error("syntax pack index is for {index_platform}, expected {expected_platform}")]
    PlatformMismatch {
        index_platform: String,
        expected_platform: String,
    },
    #[error("syntax pack index tree-sitter ABI is {index_abi}, expected {expected_abi}")]
    AbiMismatch { index_abi: u32, expected_abi: u32 },
    #[error("no syntax pack for {0}")]
    MissingLanguage(LanguageId),
    #[error("syntax pack file {path} failed checksum verification")]
    ChecksumMismatch { path: PathBuf },
    #[error("syntax pack manifest for {expected} described {actual}")]
    ManifestLanguageMismatch { expected: String, actual: String },
    #[error("syntax pack manifest is incomplete for {0}")]
    IncompleteManifest(LanguageId),
    #[error("syntax pack network request failed: {0}")]
    Network(#[from] reqwest::Error),
    #[error("syntax pack I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("syntax pack JSON failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("syntax pack dynamic library load failed: {0}")]
    Library(#[from] libloading::Error),
    #[error("invalid hex in syntax pack metadata")]
    InvalidHex,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SignedPackIndex {
    pub payload: Value,
    pub signature: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PackIndex {
    pub schema_version: u32,
    pub generated_from: String,
    pub generated_at: String,
    pub platform: String,
    pub tree_sitter_abi: u32,
    pub packs: Vec<PackIndexEntry>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PackIndexEntry {
    pub language: String,
    pub version: String,
    pub common: bool,
    pub extensions: Vec<String>,
    pub symbol: String,
    pub manifest: RemotePackFile,
    pub parser: RemotePackFile,
    pub highlights: RemotePackFile,
    pub injections: Option<RemotePackFile>,
    pub source: PackSource,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RemotePackFile {
    pub url: String,
    pub path: String,
    pub sha256: String,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct PackSource {
    pub registry_url: Option<String>,
    pub parser_url: Option<String>,
    pub query_url: Option<String>,
    pub revision: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PackManifest {
    pub schema_version: u32,
    pub language: String,
    pub version: String,
    pub platform: String,
    pub tree_sitter_abi: u32,
    pub symbol: String,
    pub parser: LocalPackFile,
    pub highlights: LocalPackFile,
    pub injections: Option<LocalPackFile>,
    pub extensions: Vec<String>,
    pub source: PackSource,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LocalPackFile {
    pub path: String,
    pub sha256: String,
}

pub(crate) struct LoadedPack {
    pub(crate) language: ts::Language,
    pub(crate) query_fragments: Vec<String>,
    pub(crate) _library: Rc<Library>,
}

#[derive(Debug, Clone)]
pub struct PackInstaller {
    index_url: String,
    storage_dir: PathBuf,
}

impl PackInstaller {
    pub fn new() -> Result<Self> {
        Ok(Self {
            index_url: default_index_url(),
            storage_dir: default_storage_dir()?,
        })
    }

    pub fn with_storage_dir(storage_dir: PathBuf) -> Self {
        Self {
            index_url: default_index_url(),
            storage_dir,
        }
    }

    pub fn with_index_url(mut self, index_url: impl Into<String>) -> Self {
        self.index_url = index_url.into();
        self
    }

    pub fn storage_dir(&self) -> &Path {
        &self.storage_dir
    }

    pub async fn install_common_packs(&self) -> Result<Vec<LanguageId>> {
        let index = self.fetch_index().await?;
        let mut installed = Vec::new();
        for entry in index.packs.iter().filter(|entry| entry.common) {
            let Some(language) = LanguageId::from_name(&entry.language) else {
                continue;
            };
            if self.install_entry(entry, language).await? {
                installed.push(language);
            }
        }
        Ok(installed)
    }

    pub async fn ensure_pack_for_path(&self, path: &Path) -> Result<Option<LanguageId>> {
        let Some(language) = crate::language::guess_language(path) else {
            return Ok(None);
        };
        if is_pack_installed_at(&self.storage_dir, language) {
            return Ok(None);
        }
        let index = self.fetch_index().await?;
        let entry = index
            .packs
            .iter()
            .find(|entry| entry.language == language.name())
            .ok_or(PackError::MissingLanguage(language))?;
        if self.install_entry(entry, language).await? {
            Ok(Some(language))
        } else {
            Ok(None)
        }
    }

    async fn fetch_index(&self) -> Result<PackIndex> {
        if !allow_unsigned_packs() && trusted_public_key_hexes().next().is_none() {
            return Err(PackError::MissingTrustedKey);
        }
        let body = reqwest::Client::new()
            .get(&self.index_url)
            .header("User-Agent", "diffy/0.1 phosphor")
            .send()
            .await?
            .error_for_status()?
            .bytes()
            .await?;
        let signed: SignedPackIndex = serde_json::from_slice(&body)?;
        verify_index_signature(&signed)?;
        let payload = serde_json::from_value(signed.payload)?;
        verify_index_compatibility(&payload)?;
        Ok(payload)
    }

    async fn install_entry(&self, entry: &PackIndexEntry, language: LanguageId) -> Result<bool> {
        if is_pack_installed_at(&self.storage_dir, language) {
            return Ok(false);
        }

        let pack_dir = self
            .storage_dir
            .join(language.name())
            .join(&entry.version)
            .join(platform_triple());
        fs::create_dir_all(&pack_dir)?;

        let manifest_bytes = download_verified(&entry.manifest).await?;
        let parser_bytes = download_verified(&entry.parser).await?;
        let highlights_bytes = download_verified(&entry.highlights).await?;
        let injections_bytes = match &entry.injections {
            Some(file) => Some(download_verified(file).await?),
            None => None,
        };

        let manifest = build_manifest(entry);
        let manifest_json = serde_json::to_vec_pretty(&manifest)?;
        if manifest_json != manifest_bytes {
            let parsed: PackManifest = serde_json::from_slice(&manifest_bytes)?;
            validate_manifest(language, &parsed)?;
        }

        write_verified_file(
            &pack_dir.join(&entry.parser.path),
            &parser_bytes,
            &entry.parser.sha256,
        )?;
        write_verified_file(
            &pack_dir.join(&entry.highlights.path),
            &highlights_bytes,
            &entry.highlights.sha256,
        )?;
        if let (Some(file), Some(bytes)) = (&entry.injections, injections_bytes.as_deref()) {
            write_verified_file(&pack_dir.join(&file.path), bytes, &file.sha256)?;
        }
        write_verified_file(
            &pack_dir.join(&entry.manifest.path),
            &manifest_bytes,
            &entry.manifest.sha256,
        )?;

        Ok(true)
    }
}

pub(crate) fn load_pack(language: LanguageId) -> Result<Option<LoadedPack>> {
    let storage_dir = default_storage_dir()?;
    load_pack_at(&storage_dir, language)
}

pub(crate) fn is_pack_installed(language: LanguageId) -> bool {
    default_storage_dir()
        .map(|dir| is_pack_installed_at(&dir, language))
        .unwrap_or(false)
}

fn load_pack_at(storage_dir: &Path, language: LanguageId) -> Result<Option<LoadedPack>> {
    let Some((pack_dir, manifest)) = latest_manifest(storage_dir, language)? else {
        return Ok(None);
    };
    validate_manifest(language, &manifest)?;
    verify_local_file(
        &pack_dir.join(&manifest.parser.path),
        &manifest.parser.sha256,
    )?;
    verify_local_file(
        &pack_dir.join(&manifest.highlights.path),
        &manifest.highlights.sha256,
    )?;
    if let Some(injections) = &manifest.injections {
        verify_local_file(&pack_dir.join(&injections.path), &injections.sha256)?;
    }

    let library = Rc::new(unsafe { Library::new(pack_dir.join(&manifest.parser.path))? });
    type LanguageSymbol = unsafe extern "C" fn() -> *const ();
    let language_fn = unsafe {
        let symbol: libloading::Symbol<'_, LanguageSymbol> =
            library.get(manifest.symbol.as_bytes())?;
        LanguageFn::from_raw(*symbol)
    };
    let language = ts::Language::new(language_fn);
    let mut query_fragments = Vec::new();
    query_fragments.push(fs::read_to_string(
        pack_dir.join(&manifest.highlights.path),
    )?);
    if let Some(injections) = &manifest.injections {
        query_fragments.push(fs::read_to_string(pack_dir.join(&injections.path))?);
    }

    Ok(Some(LoadedPack {
        language,
        query_fragments,
        _library: library,
    }))
}

fn latest_manifest(
    storage_dir: &Path,
    language: LanguageId,
) -> Result<Option<(PathBuf, PackManifest)>> {
    let language_dir = storage_dir.join(language.name());
    if !language_dir.exists() {
        return Ok(None);
    }

    let mut candidates = Vec::new();
    for version in fs::read_dir(&language_dir)? {
        let version = version?;
        if !version.file_type()?.is_dir() {
            continue;
        }
        let pack_dir = version.path().join(platform_triple());
        let manifest_path = pack_dir.join("manifest.json");
        if !manifest_path.exists() {
            continue;
        }
        candidates.push((version.file_name(), pack_dir, manifest_path));
    }
    candidates.sort_by(|left, right| left.0.cmp(&right.0));

    let Some((_, pack_dir, manifest_path)) = candidates.pop() else {
        return Ok(None);
    };
    let manifest = serde_json::from_slice(&fs::read(manifest_path)?)?;
    Ok(Some((pack_dir, manifest)))
}

fn is_pack_installed_at(storage_dir: &Path, language: LanguageId) -> bool {
    load_pack_at(storage_dir, language).is_ok_and(|pack| pack.is_some())
}

fn build_manifest(entry: &PackIndexEntry) -> PackManifest {
    PackManifest {
        schema_version: 1,
        language: entry.language.clone(),
        version: entry.version.clone(),
        platform: platform_triple().to_owned(),
        tree_sitter_abi: expected_tree_sitter_abi(),
        symbol: entry.symbol.clone(),
        parser: LocalPackFile {
            path: entry.parser.path.clone(),
            sha256: entry.parser.sha256.clone(),
        },
        highlights: LocalPackFile {
            path: entry.highlights.path.clone(),
            sha256: entry.highlights.sha256.clone(),
        },
        injections: entry.injections.as_ref().map(|file| LocalPackFile {
            path: file.path.clone(),
            sha256: file.sha256.clone(),
        }),
        extensions: entry.extensions.clone(),
        source: entry.source.clone(),
    }
}

fn validate_manifest(language: LanguageId, manifest: &PackManifest) -> Result<()> {
    if manifest.language != language.name() {
        return Err(PackError::ManifestLanguageMismatch {
            expected: language.name().to_owned(),
            actual: manifest.language.clone(),
        });
    }
    if manifest.platform != platform_triple() {
        return Err(PackError::PlatformMismatch {
            index_platform: manifest.platform.clone(),
            expected_platform: platform_triple().to_owned(),
        });
    }
    if manifest.tree_sitter_abi != expected_tree_sitter_abi() {
        return Err(PackError::AbiMismatch {
            index_abi: manifest.tree_sitter_abi,
            expected_abi: expected_tree_sitter_abi(),
        });
    }
    if manifest.symbol.is_empty()
        || manifest.parser.path.is_empty()
        || manifest.highlights.path.is_empty()
    {
        return Err(PackError::IncompleteManifest(language));
    }
    Ok(())
}

fn verify_index_compatibility(index: &PackIndex) -> Result<()> {
    if index.platform != platform_triple() {
        return Err(PackError::PlatformMismatch {
            index_platform: index.platform.clone(),
            expected_platform: platform_triple().to_owned(),
        });
    }
    if index.tree_sitter_abi != expected_tree_sitter_abi() {
        return Err(PackError::AbiMismatch {
            index_abi: index.tree_sitter_abi,
            expected_abi: expected_tree_sitter_abi(),
        });
    }
    Ok(())
}

async fn download_verified(file: &RemotePackFile) -> Result<Vec<u8>> {
    let bytes = reqwest::Client::new()
        .get(&file.url)
        .header("User-Agent", "diffy/0.1 phosphor")
        .send()
        .await?
        .error_for_status()?
        .bytes()
        .await?
        .to_vec();
    if sha256_hex(&bytes) != normalize_hex(&file.sha256)? {
        return Err(PackError::ChecksumMismatch {
            path: PathBuf::from(&file.path),
        });
    }
    Ok(bytes)
}

fn write_verified_file(path: &Path, bytes: &[u8], sha256: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temp_path = path.with_extension("part");
    fs::write(&temp_path, bytes)?;
    verify_local_file(&temp_path, sha256)?;
    if path.exists() {
        fs::remove_file(path)?;
    }
    fs::rename(temp_path, path)?;
    Ok(())
}

fn verify_local_file(path: &Path, expected_sha256: &str) -> Result<()> {
    let bytes = fs::read(path)?;
    if sha256_hex(&bytes) == normalize_hex(expected_sha256)? {
        Ok(())
    } else {
        Err(PackError::ChecksumMismatch {
            path: path.to_path_buf(),
        })
    }
}

fn verify_index_signature(index: &SignedPackIndex) -> Result<()> {
    if allow_unsigned_packs() {
        return Ok(());
    }

    verify_index_signature_with_keys(index, trusted_public_key_hexes())
}

fn verify_index_signature_with_keys<'a>(
    index: &SignedPackIndex,
    trusted_keys: impl IntoIterator<Item = &'a str>,
) -> Result<()> {
    let signature = decode_hex(&index.signature)?;
    let payload = canonical_json_bytes(&index.payload)?;
    let mut saw_key = false;
    for key_hex in trusted_keys {
        saw_key = true;
        let key = decode_hex(key_hex)?;
        if key.len() != 32 {
            continue;
        }
        let public_key = UnparsedPublicKey::new(&ED25519, key);
        if public_key.verify(&payload, &signature).is_ok() {
            return Ok(());
        }
    }

    if saw_key {
        Err(PackError::InvalidSignature)
    } else {
        Err(PackError::MissingTrustedKey)
    }
}

fn canonical_json_bytes(value: &Value) -> Result<Vec<u8>> {
    let mut out = String::new();
    write_canonical_json(value, &mut out)?;
    Ok(out.into_bytes())
}

fn write_canonical_json(value: &Value, out: &mut String) -> Result<()> {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(value) => out.push_str(if *value { "true" } else { "false" }),
        Value::Number(value) => out.push_str(&value.to_string()),
        Value::String(value) => out.push_str(&serde_json::to_string(value)?),
        Value::Array(values) => {
            out.push('[');
            for (index, value) in values.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                write_canonical_json(value, out)?;
            }
            out.push(']');
        }
        Value::Object(values) => {
            out.push('{');
            let mut keys = values.keys().collect::<Vec<_>>();
            keys.sort();
            for (index, key) in keys.into_iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                out.push_str(&serde_json::to_string(key)?);
                out.push(':');
                write_canonical_json(&values[key], out)?;
            }
            out.push('}');
        }
    }
    Ok(())
}

fn trusted_public_key_hexes() -> impl Iterator<Item = &'static str> {
    option_env!("PHOSPHOR_PACK_INDEX_PUBLIC_KEY").into_iter()
}

fn allow_unsigned_packs() -> bool {
    std::env::var_os(ALLOW_UNSIGNED_ENV).is_some()
}

fn default_storage_dir() -> Result<PathBuf> {
    dirs::data_local_dir()
        .map(|base| base.join("diffy").join("phosphor").join("languages"))
        .ok_or(PackError::MissingStorageDir)
}

fn default_index_url() -> String {
    format!("{DEFAULT_INDEX_BASE_URL}/{}.json", platform_triple())
}

fn platform_triple() -> &'static str {
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    {
        "windows-x86_64"
    }
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        "macos-aarch64"
    }
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    {
        "macos-x86_64"
    }
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        "linux-x86_64"
    }
    #[cfg(not(any(
        all(target_os = "windows", target_arch = "x86_64"),
        all(target_os = "macos", target_arch = "aarch64"),
        all(target_os = "macos", target_arch = "x86_64"),
        all(target_os = "linux", target_arch = "x86_64")
    )))]
    {
        "unknown"
    }
}

fn expected_tree_sitter_abi() -> u32 {
    tree_sitter::LANGUAGE_VERSION as u32
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(&mut out, "{byte:02x}");
    }
    out
}

fn normalize_hex(value: &str) -> Result<String> {
    let normalized = value.trim().to_ascii_lowercase();
    if normalized.len() % 2 != 0 || !normalized.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(PackError::InvalidHex);
    }
    Ok(normalized)
}

fn decode_hex(value: &str) -> Result<Vec<u8>> {
    let value = normalize_hex(value)?;
    let mut out = Vec::with_capacity(value.len() / 2);
    let bytes = value.as_bytes();
    for pair in bytes.chunks_exact(2) {
        let high = hex_value(pair[0])?;
        let low = hex_value(pair[1])?;
        out.push((high << 4) | low);
    }
    Ok(out)
}

fn hex_value(byte: u8) -> Result<u8> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => Err(PackError::InvalidHex),
    }
}

#[cfg(test)]
mod tests {
    use ring::signature::{Ed25519KeyPair, KeyPair};
    use serde_json::json;

    use super::{
        PackError, SignedPackIndex, canonical_json_bytes, decode_hex, normalize_hex, sha256_hex,
        verify_index_signature_with_keys,
    };

    #[test]
    fn sha256_is_hex_encoded() {
        assert_eq!(
            sha256_hex(b"diffy"),
            "0a312d0e053af2745f4b41572b2b21f1a1f3d4556d3b1ba76bb28b3d688e615a"
        );
    }

    #[test]
    fn hex_decoder_rejects_bad_input() {
        assert!(normalize_hex("abc").is_err());
        assert!(decode_hex("zz").is_err());
        assert_eq!(decode_hex("0A").unwrap(), vec![10]);
    }

    #[test]
    fn canonical_json_sorts_object_keys() {
        let payload = json!({
            "z": true,
            "a": ["x", { "b": 1, "a": 2 }],
        });

        assert_eq!(
            String::from_utf8(canonical_json_bytes(&payload).unwrap()).unwrap(),
            r#"{"a":["x",{"a":2,"b":1}],"z":true}"#
        );
    }

    #[test]
    fn signed_index_verifies_canonical_payload() {
        let key_pair = Ed25519KeyPair::from_seed_unchecked(&[7; 32]).unwrap();
        let payload = json!({
            "schema_version": 1,
            "generated_from": "test",
            "generated_at": "2026-04-24T00:00:00Z",
            "platform": "windows-x86_64",
            "tree_sitter_abi": 15,
            "packs": [],
            "upstream_languages": ["rust"],
        });
        let signature = key_pair.sign(&canonical_json_bytes(&payload).unwrap());
        let signed = SignedPackIndex {
            payload,
            signature: hex(signature.as_ref()),
        };
        let public_key = hex(key_pair.public_key().as_ref());

        assert!(verify_index_signature_with_keys(&signed, [public_key.as_str()]).is_ok());
    }

    #[test]
    fn signed_index_rejects_tampered_payload() {
        let key_pair = Ed25519KeyPair::from_seed_unchecked(&[7; 32]).unwrap();
        let payload = json!({
            "schema_version": 1,
            "generated_from": "test",
            "generated_at": "2026-04-24T00:00:00Z",
            "platform": "windows-x86_64",
            "tree_sitter_abi": 15,
            "packs": [],
        });
        let signature = key_pair.sign(&canonical_json_bytes(&payload).unwrap());
        let mut signed = SignedPackIndex {
            payload,
            signature: hex(signature.as_ref()),
        };
        let public_key = hex(key_pair.public_key().as_ref());
        signed.payload["generated_from"] = json!("tampered");

        assert!(matches!(
            verify_index_signature_with_keys(&signed, [public_key.as_str()]),
            Err(PackError::InvalidSignature)
        ));
    }

    #[test]
    fn generated_index_signature_verifies_when_public_key_is_available() {
        let Some(public_key) = option_env!("PHOSPHOR_TEST_PACK_INDEX_PUBLIC_KEY") else {
            return;
        };
        let signed: SignedPackIndex =
            serde_json::from_str(include_str!("../../../assets/phosphor-index.json")).unwrap();

        verify_index_signature_with_keys(&signed, [public_key]).unwrap();
    }

    fn hex(bytes: &[u8]) -> String {
        let mut out = String::with_capacity(bytes.len() * 2);
        for byte in bytes {
            use std::fmt::Write as _;
            let _ = write!(&mut out, "{byte:02x}");
        }
        out
    }
}

use std::collections::HashMap;
use std::env;
use std::fs;
#[cfg(any(target_os = "macos", target_os = "linux"))]
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use ring::digest::{SHA256, digest};
use ring::signature::{ED25519, UnparsedPublicKey};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::core::error::{DiffyError, Result};
use crate::core::http;

const DEFAULT_MANIFEST_URL: &str =
    "https://github.com/seatedro/diffy/releases/latest/download/diffy-update.json";
const MANIFEST_URL_ENV: &str = "DIFFY_UPDATE_MANIFEST_URL";
const UPDATE_PUBLIC_KEY_ENV: &str = "DIFFY_UPDATE_PUBLIC_KEY";
const ALLOW_UNSIGNED_ENV: &str = "DIFFY_ALLOW_UNSIGNED_UPDATES";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignedUpdateManifest {
    pub payload: UpdateManifest,
    pub signature: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateManifest {
    pub version: String,
    pub pub_date: Option<String>,
    pub channel: String,
    pub notes: Option<String>,
    #[serde(default)]
    pub minimum_supported_version: Option<String>,
    pub platforms: HashMap<String, UpdatePlatform>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdatePlatform {
    pub format: UpdateFormat,
    pub url: String,
    pub signature: String,
    pub sha256: String,
    #[serde(default)]
    pub size: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UpdateFormat {
    Dmg,
    AppImage,
    Nsis,
    Deb,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AvailableUpdate {
    pub version: String,
    pub channel: String,
    pub notes: Option<String>,
    pub pub_date: Option<String>,
    pub platform: String,
    pub artifact: UpdatePlatform,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateCheck {
    Available(AvailableUpdate),
    NotAvailable,
}

pub fn check_for_update(current_version: &str) -> Result<UpdateCheck> {
    let manifest = fetch_manifest()?;
    verify_manifest_signature(&manifest)?;

    if !version_is_newer(&manifest.payload.version, current_version) {
        return Ok(UpdateCheck::NotAvailable);
    }

    let platform = platform_key();
    let Some(artifact) = manifest.payload.platforms.get(platform).cloned() else {
        return Err(DiffyError::General(format!(
            "no update artifact published for {platform}"
        )));
    };
    if !artifact_can_update_current_install(artifact.format) {
        return Ok(UpdateCheck::NotAvailable);
    }

    Ok(UpdateCheck::Available(AvailableUpdate {
        version: manifest.payload.version,
        channel: manifest.payload.channel,
        notes: manifest.payload.notes,
        pub_date: manifest.payload.pub_date,
        platform: platform.to_owned(),
        artifact,
    }))
}

pub fn updates_configured() -> bool {
    env::var_os(ALLOW_UNSIGNED_ENV).is_some()
        || trusted_update_public_keys()
            .map(|keys| !keys.is_empty())
            .unwrap_or(false)
}

pub fn download_and_install(update: &AvailableUpdate) -> Result<()> {
    let tmp = tempfile::Builder::new().prefix("diffy-update.").tempdir()?;
    let artifact_path = tmp.path().join(artifact_file_name(update));
    download_file(&update.artifact.url, &artifact_path)?;
    verify_artifact_signature(&artifact_path, &update.artifact.signature)?;
    verify_sha256(&artifact_path, &update.artifact.sha256)?;

    let persist_path = persist_update_artifact(&artifact_path)?;
    launch_platform_installer(update, &persist_path)?;
    Ok(())
}

fn fetch_manifest() -> Result<SignedUpdateManifest> {
    let url = manifest_url();
    let text = http::block_on(async {
        let response = reqwest::get(&url).await.map_err(|error| {
            DiffyError::Http(format!("update manifest request failed: {error}"))
        })?;
        http::response_text(response, "update manifest").await
    })?;
    serde_json::from_str(&text).map_err(Into::into)
}

fn download_file(url: &str, path: &Path) -> Result<()> {
    let bytes = http::block_on(async {
        let response = reqwest::get(url)
            .await
            .map_err(|error| DiffyError::Http(format!("update download failed: {error}")))?;
        http::response_bytes(response, "update download").await
    })?;
    fs::write(path, bytes)?;
    Ok(())
}

fn verify_manifest_signature(manifest: &SignedUpdateManifest) -> Result<()> {
    if env::var_os(ALLOW_UNSIGNED_ENV).is_some() {
        return Ok(());
    }

    let signature = decode_hex(&manifest.signature)?;
    let payload = canonical_json_bytes(&serde_json::to_value(&manifest.payload)?)?;
    let keys = trusted_update_public_keys()?;
    for key in &keys {
        if UnparsedPublicKey::new(&ED25519, key)
            .verify(&payload, &signature)
            .is_ok()
        {
            return Ok(());
        }
    }

    if keys.is_empty() {
        Err(DiffyError::General(format!(
            "updates are not configured; set {UPDATE_PUBLIC_KEY_ENV} at build time"
        )))
    } else {
        Err(DiffyError::General(
            "update manifest signature is invalid".to_owned(),
        ))
    }
}

fn verify_sha256(path: &Path, expected: &str) -> Result<()> {
    let bytes = fs::read(path)?;
    let actual = hex(digest(&SHA256, &bytes).as_ref());
    if actual == normalize_hex(expected)? {
        Ok(())
    } else {
        Err(DiffyError::General(format!(
            "update checksum mismatch for {}",
            path.display()
        )))
    }
}

fn verify_artifact_signature(path: &Path, signature_hex: &str) -> Result<()> {
    if env::var_os(ALLOW_UNSIGNED_ENV).is_some() {
        return Ok(());
    }

    let bytes = fs::read(path)?;
    let signature = decode_hex(signature_hex)?;
    let keys = trusted_update_public_keys()?;
    for key in &keys {
        if UnparsedPublicKey::new(&ED25519, key)
            .verify(&bytes, &signature)
            .is_ok()
        {
            return Ok(());
        }
    }

    if keys.is_empty() {
        Err(DiffyError::General(format!(
            "updates are not configured; set {UPDATE_PUBLIC_KEY_ENV} at build time"
        )))
    } else {
        Err(DiffyError::General(format!(
            "update artifact signature is invalid for {}",
            path.display()
        )))
    }
}

fn persist_update_artifact(path: &Path) -> Result<PathBuf> {
    let file_name = path
        .file_name()
        .ok_or_else(|| DiffyError::General("update artifact has no file name".to_owned()))?;
    let dir = env::temp_dir().join(format!("diffy-update-{}", std::process::id()));
    fs::create_dir_all(&dir)?;
    let dest = dir.join(file_name);
    fs::copy(path, &dest)?;
    Ok(dest)
}

fn launch_platform_installer(update: &AvailableUpdate, artifact_path: &Path) -> Result<()> {
    match (std::env::consts::OS, update.artifact.format) {
        ("macos", UpdateFormat::Dmg) => launch_macos_installer(artifact_path),
        ("linux", UpdateFormat::AppImage) => launch_linux_appimage_installer(artifact_path),
        ("windows", UpdateFormat::Nsis) => launch_windows_nsis_installer(artifact_path),
        (_, format) => Err(DiffyError::General(format!(
            "unsupported update format {format:?} for {}",
            std::env::consts::OS
        ))),
    }
}

#[cfg(target_os = "macos")]
fn launch_macos_installer(dmg_path: &Path) -> Result<()> {
    let current_exe = env::current_exe()?;
    let app_path = find_macos_app_bundle(&current_exe)?;
    let needs_admin = app_path
        .parent()
        .is_none_or(|parent| !directory_is_writable(parent));
    let script = format!(
        r#"set -eu
APP_PATH={app_path:?}
DMG_PATH={dmg_path:?}
PID={pid}
MOUNT="$(mktemp -d /tmp/diffy-update-mount.XXXXXX)"
while kill -0 "$PID" 2>/dev/null; do sleep 0.2; done
hdiutil attach "$DMG_PATH" -mountpoint "$MOUNT" -nobrowse -quiet -readonly
SRC="$MOUNT/Diffy.app"
if [ ! -d "$SRC" ]; then
  hdiutil detach "$MOUNT" -quiet -force || true
  exit 1
fi
rm -rf "$APP_PATH"
ditto "$SRC" "$APP_PATH"
/usr/bin/xattr -cr "$APP_PATH" || true
hdiutil detach "$MOUNT" -quiet -force || true
open "$APP_PATH"
"#,
        app_path = app_path.to_string_lossy(),
        dmg_path = dmg_path.to_string_lossy(),
        pid = std::process::id()
    );
    spawn_macos_installer_script(script, needs_admin)?;
    std::process::exit(0);
}

#[cfg(not(target_os = "macos"))]
fn launch_macos_installer(_dmg_path: &Path) -> Result<()> {
    Err(DiffyError::General(
        "macOS updates are only supported on macOS".to_owned(),
    ))
}

#[cfg(target_os = "linux")]
fn launch_linux_appimage_installer(appimage_path: &Path) -> Result<()> {
    let dest = env::var_os("APPIMAGE").map(PathBuf::from).ok_or_else(|| {
        DiffyError::General("AppImage updates require running Diffy from an AppImage".to_owned())
    })?;
    let script = format!(
        r#"set -eu
DEST={dest:?}
SRC={src:?}
PID={pid}
while kill -0 "$PID" 2>/dev/null; do sleep 0.2; done
chmod 0755 "$SRC"
mv "$SRC" "$DEST"
chmod 0755 "$DEST"
"$DEST" >/dev/null 2>&1 &
"#,
        dest = dest.to_string_lossy(),
        src = appimage_path.to_string_lossy(),
        pid = std::process::id()
    );
    spawn_shell_script(script)?;
    std::process::exit(0);
}

#[cfg(not(target_os = "linux"))]
fn launch_linux_appimage_installer(_appimage_path: &Path) -> Result<()> {
    Err(DiffyError::General(
        "AppImage updates are only supported on Linux".to_owned(),
    ))
}

#[cfg(target_os = "windows")]
fn launch_windows_nsis_installer(setup_path: &Path) -> Result<()> {
    let script = format!(
        r#"$ErrorActionPreference = "Stop"
$pidToWait = {pid}
$setup = {setup:?}
Wait-Process -Id $pidToWait -ErrorAction SilentlyContinue
$proc = Start-Process -FilePath $setup -ArgumentList "/S" -Wait -PassThru
if ($proc.ExitCode -ne 0) {{ exit $proc.ExitCode }}
$app = Join-Path $env:LOCALAPPDATA "Diffy\Diffy.exe"
if (Test-Path $app) {{ Start-Process -FilePath $app }}
"#,
        setup = setup_path.to_string_lossy(),
        pid = std::process::id()
    );
    let mut child = Command::new("powershell")
        .args([
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            &script,
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| DiffyError::General(format!("failed to launch updater: {error}")))?;
    let _ = child.stdin.take();
    std::process::exit(0);
}

#[cfg(not(target_os = "windows"))]
fn launch_windows_nsis_installer(_setup_path: &Path) -> Result<()> {
    Err(DiffyError::General(
        "NSIS updates are only supported on Windows".to_owned(),
    ))
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn spawn_shell_script(script: String) -> Result<()> {
    let script_path = env::temp_dir().join(format!("diffy-update-{}.sh", std::process::id()));
    let mut file = fs::File::create(&script_path)?;
    file.write_all(script.as_bytes())?;
    drop(file);
    Command::new("sh")
        .arg(script_path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| DiffyError::General(format!("failed to launch updater: {error}")))?;
    Ok(())
}

#[cfg(target_os = "macos")]
fn spawn_macos_installer_script(script: String, needs_admin: bool) -> Result<()> {
    if !needs_admin {
        return spawn_shell_script(script);
    }

    let script_path = env::temp_dir().join(format!("diffy-update-{}.sh", std::process::id()));
    let mut file = fs::File::create(&script_path)?;
    file.write_all(script.as_bytes())?;
    drop(file);
    let command = format!("/bin/sh {}", shell_quote(&script_path.to_string_lossy()));
    let apple_script = format!(
        "do shell script {} with administrator privileges",
        apple_script_string(&command)
    );
    Command::new("osascript")
        .args(["-e", &apple_script])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| {
            DiffyError::General(format!("failed to launch privileged updater: {error}"))
        })?;
    Ok(())
}

#[cfg(target_os = "macos")]
fn directory_is_writable(path: &Path) -> bool {
    let probe = path.join(format!(".diffy-update-write-test-{}", std::process::id()));
    match fs::File::create(&probe) {
        Ok(_) => {
            let _ = fs::remove_file(probe);
            true
        }
        Err(_) => false,
    }
}

#[cfg(target_os = "macos")]
fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[cfg(target_os = "macos")]
fn apple_script_string(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

#[cfg(target_os = "macos")]
fn find_macos_app_bundle(exe: &Path) -> Result<PathBuf> {
    let mut cursor = Some(exe);
    while let Some(path) = cursor {
        if path.extension().is_some_and(|ext| ext == "app") {
            return Ok(path.to_path_buf());
        }
        cursor = path.parent();
    }
    Err(DiffyError::General(format!(
        "could not find .app bundle for {}",
        exe.display()
    )))
}

fn artifact_file_name(update: &AvailableUpdate) -> String {
    update
        .artifact
        .url
        .rsplit('/')
        .next()
        .filter(|name| !name.is_empty())
        .unwrap_or("diffy-update")
        .to_owned()
}

fn manifest_url() -> String {
    if let Some(url) = option_env!("DIFFY_UPDATE_MANIFEST_URL") {
        url.to_owned()
    } else {
        env::var(MANIFEST_URL_ENV).unwrap_or_else(|_| DEFAULT_MANIFEST_URL.to_owned())
    }
}

fn configured_update_public_key_hexes() -> Vec<String> {
    let mut keys = Vec::new();
    if let Some(key) = option_env!("DIFFY_UPDATE_PUBLIC_KEY") {
        let key = key.trim();
        if !key.is_empty() {
            keys.push(key.to_owned());
        }
    }
    if let Ok(key) = env::var(UPDATE_PUBLIC_KEY_ENV)
        && !key.trim().is_empty()
    {
        let key = key.trim().to_owned();
        if !keys.iter().any(|embedded| embedded == &key) {
            keys.push(key);
        }
    }
    keys
}

fn trusted_update_public_keys() -> Result<Vec<Vec<u8>>> {
    configured_update_public_key_hexes()
        .into_iter()
        .map(|key_hex| {
            let key = decode_hex(&key_hex)?;
            if key.len() == 32 {
                Ok(key)
            } else {
                Err(DiffyError::General(format!(
                    "{UPDATE_PUBLIC_KEY_ENV} must contain 32-byte Ed25519 public keys as hex"
                )))
            }
        })
        .collect()
}

fn artifact_can_update_current_install(format: UpdateFormat) -> bool {
    match (std::env::consts::OS, format) {
        ("linux", UpdateFormat::AppImage) => env::var_os("APPIMAGE").is_some(),
        _ => true,
    }
}

pub fn platform_key() -> &'static str {
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    return "windows-x86_64";
    #[cfg(all(target_os = "windows", target_arch = "aarch64"))]
    return "windows-aarch64";
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    return "macos-aarch64";
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    return "macos-x86_64";
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    return "linux-x86_64";
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    return "linux-aarch64";
    #[allow(unreachable_code)]
    "unknown"
}

fn version_is_newer(candidate: &str, current: &str) -> bool {
    parse_version(candidate) > parse_version(current)
}

fn parse_version(value: &str) -> (u64, u64, u64, bool) {
    let value = value.trim_start_matches('v');
    let (core, pre) = value
        .split_once('-')
        .map_or((value, false), |(core, _)| (core, true));
    let mut parts = core.split('.').map(|p| p.parse::<u64>().unwrap_or(0));
    (
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
        !pre,
    )
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

fn decode_hex(value: &str) -> Result<Vec<u8>> {
    let value = normalize_hex(value)?;
    let mut out = Vec::with_capacity(value.len() / 2);
    for pair in value.as_bytes().chunks_exact(2) {
        out.push((hex_value(pair[0])? << 4) | hex_value(pair[1])?);
    }
    Ok(out)
}

fn normalize_hex(value: &str) -> Result<String> {
    let normalized = value.trim().to_ascii_lowercase();
    if normalized.len() % 2 != 0 || !normalized.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(DiffyError::Parse("invalid hex value".to_owned()));
    }
    Ok(normalized)
}

fn hex_value(byte: u8) -> Result<u8> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => Err(DiffyError::Parse("invalid hex value".to_owned())),
    }
}

fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(&mut out, "{byte:02x}");
    }
    out
}

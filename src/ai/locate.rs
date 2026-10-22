use std::collections::HashSet;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::Value;

use super::cli::{self, CliAuth, CliEnv, CliInfo, CliScan, CliTool};

const SHELL_TIMEOUT: Duration = Duration::from_secs(5);
const PROBE_TIMEOUT: Duration = Duration::from_secs(5);
const POLL: Duration = Duration::from_millis(20);
const DRAIN_WAIT: Duration = Duration::from_millis(500);
const ENV_MARKER: &str = "__DIFFY_ENV__";
const SHELL_VARS: &[&str] = &["CLAUDE_CONFIG_DIR", "CODEX_HOME"];

struct Captured {
    success: bool,
    stdout: String,
    stderr: String,
}

pub fn scan() -> CliScan {
    let shell = login_shell_env();
    let shell_path = shell
        .iter()
        .find(|(key, _)| key == "PATH")
        .map(|(_, value)| value.as_str());
    let env = CliEnv {
        path: search_path(shell_path),
        vars: shell
            .iter()
            .filter(|(key, _)| SHELL_VARS.contains(&key.as_str()))
            .map(|(key, value)| (key.clone(), OsString::from(value)))
            .collect(),
    };
    let cwd = cli::workdir()
        .filter(|dir| std::fs::create_dir_all(dir).is_ok())
        .unwrap_or_else(std::env::temp_dir);
    let tools = CliTool::ALL
        .into_iter()
        .filter_map(|tool| {
            let program = find(tool.binary(), &env.path)?;
            let version = probe(&program, &["--version"], &env, &cwd)
                .and_then(|out| parse_version(&out.stdout));
            let auth = match tool {
                CliTool::ClaudeCode => probe(&program, &["auth", "status"], &env, &cwd)
                    .map_or(CliAuth::Unknown, |out| parse_claude_auth(&out.stdout)),
                CliTool::Codex => probe(&program, &["login", "status"], &env, &cwd)
                    .map_or(CliAuth::Unknown, |out| {
                        parse_codex_auth(out.success, &format!("{}\n{}", out.stdout, out.stderr))
                    }),
            };
            Some(CliInfo {
                tool,
                program,
                version,
                auth,
            })
        })
        .collect();
    CliScan { env, tools }
}

#[cfg(unix)]
fn login_shell_env() -> Vec<(String, String)> {
    let shell = std::env::var_os("SHELL")
        .filter(|shell| !shell.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(if cfg!(target_os = "macos") {
                "/bin/zsh"
            } else {
                "/bin/sh"
            })
        });
    let script = format!("echo {ENV_MARKER}; /usr/bin/env; echo {ENV_MARKER}");
    let mut command = Command::new(&shell);
    command.arg("-ilc").arg(script).env_clear();
    for key in ["HOME", "USER", "LOGNAME", "SHELL", "TMPDIR", "LANG"] {
        if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }
    command.env("TERM", "dumb");
    match capture(&mut command, SHELL_TIMEOUT) {
        Some(out) => parse_env(&out.stdout),
        None => {
            tracing::warn!(shell = %shell.display(), "ai.cli: login shell env unavailable");
            Vec::new()
        }
    }
}

#[cfg(not(unix))]
fn login_shell_env() -> Vec<(String, String)> {
    Vec::new()
}

fn parse_env(output: &str) -> Vec<(String, String)> {
    let mut sections = output.split(ENV_MARKER);
    let (Some(_), Some(body), Some(_)) = (sections.next(), sections.next(), sections.next()) else {
        return Vec::new();
    };
    body.lines()
        .filter_map(|line| line.split_once('='))
        .filter(|(key, _)| *key == "PATH" || SHELL_VARS.contains(key))
        .map(|(key, value)| (key.to_owned(), value.to_owned()))
        .collect()
}

fn search_path(shell_path: Option<&str>) -> OsString {
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Some(path) = shell_path {
        dirs.extend(std::env::split_paths(path));
    }
    if let Some(path) = std::env::var_os("PATH") {
        dirs.extend(std::env::split_paths(&path));
    }
    dirs.extend(common_dirs());
    let mut seen = HashSet::new();
    let dirs: Vec<PathBuf> = dirs
        .into_iter()
        .filter(|dir| dir.is_absolute() && std::env::join_paths([dir]).is_ok())
        .filter(|dir| seen.insert(dir.clone()))
        .collect();
    std::env::join_paths(dirs).unwrap_or_default()
}

fn common_dirs() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(home) = dirs::home_dir() {
        for dir in [
            ".local/bin",
            ".claude/local",
            ".npm-global/bin",
            ".bun/bin",
            ".volta/bin",
            ".yarn/bin",
            ".local/share/pnpm",
            "Library/pnpm",
            ".local/share/fnm/aliases/default/bin",
            ".local/share/mise/shims",
            ".asdf/shims",
            ".nix-profile/bin",
            ".cargo/bin",
        ] {
            out.push(home.join(dir));
        }
        out.extend(nvm_bins(&home.join(".nvm/versions/node")));
        if cfg!(windows) {
            out.push(home.join("AppData/Roaming/npm"));
        }
    }
    for dir in [
        "/opt/homebrew/bin",
        "/usr/local/bin",
        "/usr/bin",
        "/bin",
        "/usr/sbin",
        "/sbin",
    ] {
        out.push(PathBuf::from(dir));
    }
    out
}

fn nvm_bins(root: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut versions: Vec<(Vec<u32>, PathBuf)> = entries
        .filter_map(|entry| entry.ok())
        .map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            let key = name
                .trim_start_matches('v')
                .split('.')
                .map(|part| part.parse().unwrap_or(0))
                .collect();
            (key, entry.path().join("bin"))
        })
        .collect();
    versions.sort_by(|a, b| b.0.cmp(&a.0));
    versions.into_iter().map(|(_, bin)| bin).collect()
}

fn find(binary: &str, path: &OsString) -> Option<PathBuf> {
    std::env::split_paths(path)
        .flat_map(|dir| candidates(&dir, binary))
        .find(|candidate| executable(candidate))
}

#[cfg(unix)]
fn candidates(dir: &Path, binary: &str) -> Vec<PathBuf> {
    vec![dir.join(binary)]
}

#[cfg(not(unix))]
fn candidates(dir: &Path, binary: &str) -> Vec<PathBuf> {
    ["exe", "cmd"]
        .into_iter()
        .map(|ext| dir.join(binary).with_extension(ext))
        .collect()
}

#[cfg(unix)]
fn executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn executable(path: &Path) -> bool {
    path.is_file()
}

fn probe(program: &Path, args: &[&str], env: &CliEnv, cwd: &Path) -> Option<Captured> {
    let mut command = Command::new(program);
    command.args(args).current_dir(cwd);
    env.apply(&mut command);
    capture(&mut command, PROBE_TIMEOUT)
}

fn capture(command: &mut Command, timeout: Duration) -> Option<Captured> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    cli::isolate(command);
    crate::platform::energy::count_process();
    let mut child = command.spawn().ok()?;
    let (Some(stdout), Some(stderr)) = (child.stdout.take(), child.stderr.take()) else {
        cli::stop(&mut child);
        return None;
    };
    let stdout = cli::read_all(stdout);
    let stderr = cli::read_all(stderr);
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => thread::sleep(POLL),
            _ => {
                cli::stop(&mut child);
                return None;
            }
        }
    };
    Some(Captured {
        success: status.success(),
        stdout: cli::drain(&child, &stdout, DRAIN_WAIT),
        stderr: cli::drain(&child, &stderr, DRAIN_WAIT),
    })
}

fn parse_version(output: &str) -> Option<String> {
    output
        .lines()
        .next()?
        .split_whitespace()
        .find(|token| token.starts_with(|c: char| c.is_ascii_digit()))
        .map(|token| {
            token
                .trim_end_matches(|c: char| !c.is_ascii_alphanumeric())
                .to_owned()
        })
}

fn parse_claude_auth(stdout: &str) -> CliAuth {
    let logged_in = serde_json::from_str::<Value>(stdout.trim())
        .ok()
        .and_then(|value| value.get("loggedIn").and_then(Value::as_bool));
    match logged_in {
        Some(true) => CliAuth::SignedIn,
        Some(false) => CliAuth::SignedOut,
        None => CliAuth::Unknown,
    }
}

fn parse_codex_auth(success: bool, output: &str) -> CliAuth {
    if output.contains("Not logged in") {
        CliAuth::SignedOut
    } else if success && output.contains("Logged in") {
        CliAuth::SignedIn
    } else {
        CliAuth::Unknown
    }
}

#[cfg(test)]
mod tests {
    use super::{CliAuth, nvm_bins, parse_claude_auth, parse_codex_auth, parse_env, parse_version};

    #[test]
    fn reads_versions_from_both_clis() {
        assert_eq!(
            parse_version("2.1.0 (Claude Code)\n").as_deref(),
            Some("2.1.0")
        );
        assert_eq!(parse_version("codex-cli 0.9.1").as_deref(), Some("0.9.1"));
        assert_eq!(parse_version("unknown"), None);
    }

    #[test]
    fn reads_sign_in_state() {
        assert_eq!(
            parse_claude_auth(r#"{"loggedIn": true, "authMethod": "claude.ai"}"#),
            CliAuth::SignedIn
        );
        assert_eq!(
            parse_claude_auth(r#"{"loggedIn": false}"#),
            CliAuth::SignedOut
        );
        assert_eq!(
            parse_claude_auth("error: unknown command"),
            CliAuth::Unknown
        );
        assert_eq!(
            parse_codex_auth(true, "\nLogged in using ChatGPT"),
            CliAuth::SignedIn
        );
        assert_eq!(parse_codex_auth(false, "Not logged in"), CliAuth::SignedOut);
        assert_eq!(parse_codex_auth(false, "error"), CliAuth::Unknown);
    }

    #[test]
    fn keeps_path_from_between_the_markers() {
        let output = "welcome\n__DIFFY_ENV__\nHOME=/home/a\nPATH=/a/bin:/b/bin\nCODEX_HOME=/c\n__DIFFY_ENV__\nPATH=/late\n";
        assert_eq!(
            parse_env(output),
            vec![
                ("PATH".to_owned(), "/a/bin:/b/bin".to_owned()),
                ("CODEX_HOME".to_owned(), "/c".to_owned()),
            ]
        );
        assert!(parse_env("PATH=/only").is_empty());
    }

    #[test]
    fn prefers_newer_nvm_versions() {
        let root = tempfile::tempdir().unwrap();
        for version in ["v9.1.0", "v20.3.1", "v18.0.0"] {
            std::fs::create_dir(root.path().join(version)).unwrap();
        }
        let bins = nvm_bins(root.path());
        let names: Vec<_> = bins
            .iter()
            .map(|bin| bin.parent().unwrap().file_name().unwrap().to_owned())
            .collect();
        assert_eq!(names, ["v20.3.1", "v18.0.0", "v9.1.0"]);
    }
}

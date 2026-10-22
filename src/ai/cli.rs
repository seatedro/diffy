use std::ffi::OsString;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::Value;

use super::Provider;

const SYSTEM_PROMPT: &str = "You are a helpful assistant. Reply with plain text only.";
const CLAUDE_MODEL: &str = "haiku";
const CODEX_EFFORT: &str = "model_reasoning_effort=\"low\"";
const WORKDIR: &str = "diffy";
const RUN_TIMEOUT: Duration = Duration::from_secs(180);
const POLL: Duration = Duration::from_millis(100);
const OUTPUT_LIMIT: usize = 64 * 1024;
const DRAIN_WAIT: Duration = Duration::from_secs(1);

const PASSTHROUGH_ENV: &[&str] = &[
    "HOME",
    "USER",
    "LOGNAME",
    "TMPDIR",
    "TMP",
    "TEMP",
    "LANG",
    "LC_ALL",
    "LC_CTYPE",
    "SHELL",
    "XDG_CONFIG_HOME",
    "XDG_DATA_HOME",
    "XDG_STATE_HOME",
    "XDG_CACHE_HOME",
    "XDG_RUNTIME_DIR",
    "DBUS_SESSION_BUS_ADDRESS",
    "CLAUDE_CONFIG_DIR",
    "CODEX_HOME",
    "HTTPS_PROXY",
    "HTTP_PROXY",
    "NO_PROXY",
    "ALL_PROXY",
    "https_proxy",
    "http_proxy",
    "no_proxy",
    "all_proxy",
    "NODE_EXTRA_CA_CERTS",
    "SSL_CERT_FILE",
    "SSL_CERT_DIR",
    "SYSTEMROOT",
    "SYSTEMDRIVE",
    "WINDIR",
    "COMSPEC",
    "PATHEXT",
    "USERPROFILE",
    "APPDATA",
    "LOCALAPPDATA",
    "PROGRAMDATA",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CliTool {
    ClaudeCode,
    Codex,
}

impl CliTool {
    pub const ALL: [Self; 2] = [Self::ClaudeCode, Self::Codex];

    pub fn label(self) -> &'static str {
        match self {
            Self::ClaudeCode => "Claude Code",
            Self::Codex => "Codex",
        }
    }

    pub fn binary(self) -> &'static str {
        match self {
            Self::ClaudeCode => "claude",
            Self::Codex => "codex",
        }
    }

    pub fn for_provider(provider: Provider) -> Self {
        match provider {
            Provider::Anthropic => Self::ClaudeCode,
            Provider::OpenAi => Self::Codex,
        }
    }

    pub fn sign_in_hint(self) -> &'static str {
        match self {
            Self::ClaudeCode => "Run `claude` in a terminal and sign in.",
            Self::Codex => "Run `codex login` in a terminal.",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CliAuth {
    SignedIn,
    SignedOut,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliInfo {
    pub tool: CliTool,
    pub program: PathBuf,
    pub version: Option<String>,
    pub auth: CliAuth,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CliEnv {
    pub path: OsString,
    pub vars: Vec<(String, OsString)>,
}

impl CliEnv {
    pub fn apply(&self, command: &mut Command) {
        command.env_clear();
        for key in PASSTHROUGH_ENV {
            if let Some(value) = std::env::var_os(key) {
                command.env(key, value);
            }
        }
        for (key, value) in &self.vars {
            if std::env::var_os(key).is_none() {
                command.env(key, value);
            }
        }
        command.env("PATH", &self.path).env("NO_COLOR", "1");
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CliScan {
    pub env: CliEnv,
    pub tools: Vec<CliInfo>,
}

impl CliScan {
    pub fn find(&self, tool: CliTool) -> Option<&CliInfo> {
        self.tools.iter().find(|info| info.tool == tool)
    }

    pub fn command(&self, tool: CliTool) -> Option<CliCommand> {
        self.find(tool).map(|info| CliCommand {
            tool,
            program: info.program.clone(),
            env: self.env.clone(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliCommand {
    pub tool: CliTool,
    pub program: PathBuf,
    pub env: CliEnv,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunOutcome {
    Finished,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum CliEvent {
    Text(String),
    Warning(String),
    Failed(String),
}

pub fn workdir() -> Option<PathBuf> {
    crate::platform::startup::app_config_dir().map(|dir| dir.join(WORKDIR))
}

fn args(tool: CliTool, workdir: &Path) -> Vec<OsString> {
    let fixed: &[&str] = match tool {
        CliTool::ClaudeCode => &[
            "-p",
            "--output-format",
            "stream-json",
            "--verbose",
            "--include-partial-messages",
            "--model",
            CLAUDE_MODEL,
            "--tools",
            "",
            "--strict-mcp-config",
            "--setting-sources",
            "",
            "--disable-slash-commands",
            "--no-session-persistence",
            "--system-prompt",
            SYSTEM_PROMPT,
        ],
        CliTool::Codex => &[
            "exec",
            "--json",
            "--ephemeral",
            "--skip-git-repo-check",
            "--sandbox",
            "read-only",
            "--ignore-user-config",
            "--ignore-rules",
            "--color",
            "never",
            "-c",
            CODEX_EFFORT,
        ],
    };
    let mut args: Vec<OsString> = fixed.iter().map(OsString::from).collect();
    if tool == CliTool::Codex {
        args.push("--cd".into());
        args.push(workdir.into());
        args.push("-".into());
    }
    args
}

pub fn run(
    command: &CliCommand,
    workdir: &Path,
    user_message: &str,
    live: &dyn Fn() -> bool,
    emit: &mut dyn FnMut(String) -> bool,
) -> Result<RunOutcome, String> {
    let label = command.tool.label();
    std::fs::create_dir_all(workdir)
        .map_err(|error| format!("couldn't create {}: {error}", workdir.display()))?;
    let mut process = Command::new(&command.program);
    process
        .args(args(command.tool, workdir))
        .current_dir(workdir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command.env.apply(&mut process);
    isolate(&mut process);
    crate::platform::energy::count_process();
    let mut child = process
        .spawn()
        .map_err(|error| format!("couldn't start {label}: {error}"))?;
    let (Some(mut stdin), Some(stdout), Some(stderr)) =
        (child.stdin.take(), child.stdout.take(), child.stderr.take())
    else {
        stop(&mut child);
        return Err(format!("couldn't connect to {label}"));
    };

    let input = user_message.to_owned();
    thread::spawn(move || {
        let _ = stdin.write_all(input.as_bytes());
    });
    let errors = read_all(stderr);
    let (tx, rx) = mpsc::channel();
    let tool = command.tool;
    thread::spawn(move || {
        let mut parser = StreamParser::new(tool);
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else {
                break;
            };
            if let Some(event) = parser.line(&line)
                && tx.send(event).is_err()
            {
                break;
            }
        }
    });

    let deadline = Instant::now() + RUN_TIMEOUT;
    let mut failure = None;
    let mut warning = None;
    let mut exited_at = None;
    let status = loop {
        let closed = match rx.recv_timeout(POLL) {
            Ok(CliEvent::Text(text)) => {
                if !live() || !emit(text) {
                    stop(&mut child);
                    return Ok(RunOutcome::Cancelled);
                }
                continue;
            }
            Ok(CliEvent::Warning(message)) => {
                warning = Some(message);
                continue;
            }
            Ok(CliEvent::Failed(message)) => {
                failure = Some(message);
                continue;
            }
            Err(RecvTimeoutError::Timeout) => false,
            Err(RecvTimeoutError::Disconnected) => true,
        };
        if !live() {
            stop(&mut child);
            return Ok(RunOutcome::Cancelled);
        }
        if Instant::now() >= deadline {
            stop(&mut child);
            return Err(timed_out(label));
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                let since = *exited_at.get_or_insert_with(Instant::now);
                if closed {
                    break status;
                }
                if since.elapsed() >= DRAIN_WAIT {
                    kill_group(&child);
                    break status;
                }
            }
            Ok(None) if closed => thread::sleep(POLL),
            Ok(None) => {}
            Err(error) => {
                stop(&mut child);
                return Err(format!("{label} stopped unexpectedly: {error}"));
            }
        }
    };
    let stderr = drain(&child, &errors, DRAIN_WAIT);
    if let Some(message) = failure {
        return Err(format!("{label}: {message}"));
    }
    if !status.success() {
        return Err(exit_message(label, status, warning, &stderr));
    }
    Ok(RunOutcome::Finished)
}

fn timed_out(label: &str) -> String {
    format!(
        "{label} didn't answer within {} seconds",
        RUN_TIMEOUT.as_secs()
    )
}

fn exit_message(label: &str, status: ExitStatus, warning: Option<String>, stderr: &str) -> String {
    let detail = warning.or_else(|| {
        stderr
            .lines()
            .map(str::trim)
            .rfind(|line| !line.is_empty())
            .map(str::to_owned)
    });
    match detail {
        Some(detail) => format!("{label} failed: {detail}"),
        None => format!("{label} exited with {status}"),
    }
}

pub(super) fn read_all(stream: impl Read + Send + 'static) -> mpsc::Receiver<String> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = BufReader::new(stream).read_to_end(&mut bytes);
        let start = bytes.len().saturating_sub(OUTPUT_LIMIT);
        let _ = tx.send(String::from_utf8_lossy(&bytes[start..]).into_owned());
    });
    rx
}

pub(super) fn drain(child: &Child, output: &mpsc::Receiver<String>, wait: Duration) -> String {
    output.recv_timeout(wait).unwrap_or_else(|_| {
        kill_group(child);
        output.recv_timeout(wait).unwrap_or_default()
    })
}

#[cfg(unix)]
pub(super) fn isolate(command: &mut Command) {
    use std::os::unix::process::CommandExt;
    command.process_group(0);
}

#[cfg(not(unix))]
pub(super) fn isolate(_: &mut Command) {}

pub(super) fn stop(child: &mut Child) {
    kill_group(child);
    let _ = child.kill();
    let _ = child.wait();
}

#[cfg(unix)]
fn kill_group(child: &Child) {
    if let Ok(pid) = i32::try_from(child.id()) {
        unsafe {
            libc::kill(-pid, libc::SIGKILL);
        }
    }
}

#[cfg(not(unix))]
fn kill_group(_: &Child) {}

struct StreamParser {
    tool: CliTool,
    streamed: bool,
    message: Option<String>,
}

impl StreamParser {
    fn new(tool: CliTool) -> Self {
        Self {
            tool,
            streamed: false,
            message: None,
        }
    }

    fn line(&mut self, line: &str) -> Option<CliEvent> {
        let line = line.trim();
        if line.is_empty() {
            return None;
        }
        let value: Value = serde_json::from_str(line).ok()?;
        let kind = value.get("type").and_then(Value::as_str)?;
        match self.tool {
            CliTool::ClaudeCode => self.claude(kind, &value),
            CliTool::Codex => self.codex(kind, &value),
        }
    }

    fn claude(&mut self, kind: &str, value: &Value) -> Option<CliEvent> {
        match kind {
            "stream_event" => {
                let event = value.get("event")?;
                if event.get("type").and_then(Value::as_str) != Some("content_block_delta") {
                    return None;
                }
                let delta = event.get("delta")?;
                if delta.get("type").and_then(Value::as_str) != Some("text_delta") {
                    return None;
                }
                let text = delta
                    .get("text")
                    .and_then(Value::as_str)
                    .filter(|text| !text.is_empty())?;
                self.streamed = true;
                Some(CliEvent::Text(text.to_owned()))
            }
            "result" => {
                let result = value
                    .get("result")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|result| !result.is_empty());
                if value
                    .get("is_error")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
                {
                    let errors = value
                        .get("errors")
                        .and_then(Value::as_array)
                        .map(|errors| {
                            errors
                                .iter()
                                .filter_map(Value::as_str)
                                .collect::<Vec<_>>()
                                .join("; ")
                        })
                        .filter(|errors| !errors.is_empty());
                    let message = result
                        .map(str::to_owned)
                        .or(errors)
                        .unwrap_or_else(|| "the request failed".to_owned());
                    return Some(CliEvent::Failed(message));
                }
                if self.streamed {
                    return None;
                }
                result.map(|result| CliEvent::Text(result.to_owned()))
            }
            _ => None,
        }
    }

    fn codex(&mut self, kind: &str, value: &Value) -> Option<CliEvent> {
        match kind {
            "item.completed" => {
                let item = value.get("item")?;
                if item.get("type").and_then(Value::as_str) == Some("agent_message") {
                    self.message = item.get("text").and_then(Value::as_str).map(str::to_owned);
                }
                None
            }
            "turn.completed" => self
                .message
                .take()
                .filter(|message| !message.trim().is_empty())
                .map(CliEvent::Text),
            "turn.failed" => {
                let message = value
                    .get("error")
                    .and_then(|error| error.get("message"))
                    .and_then(Value::as_str)
                    .unwrap_or("the turn failed");
                Some(CliEvent::Failed(codex_message(message)))
            }
            "error" => value
                .get("message")
                .and_then(Value::as_str)
                .map(|message| CliEvent::Warning(codex_message(message))),
            _ => None,
        }
    }
}

fn codex_message(raw: &str) -> String {
    serde_json::from_str::<Value>(raw)
        .ok()
        .and_then(|value| {
            value
                .get("error")
                .and_then(|error| error.get("message"))
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .unwrap_or_else(|| raw.to_owned())
}

#[cfg(test)]
mod tests {
    use super::{CliEvent, CliTool, StreamParser, args};
    use std::path::Path;

    fn feed(tool: CliTool, lines: &[&str]) -> Vec<CliEvent> {
        let mut parser = StreamParser::new(tool);
        lines.iter().filter_map(|line| parser.line(line)).collect()
    }

    #[test]
    fn claude_streams_text_deltas_and_skips_the_final_result() {
        let events = feed(
            CliTool::ClaudeCode,
            &[
                r#"{"type":"system","subtype":"init","session_id":"s1"}"#,
                r#"{"type":"stream_event","event":{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}}"#,
                r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"feat: add "}}}"#,
                r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"parser"}}}"#,
                r#"{"type":"assistant","message":{"content":[{"type":"text","text":"feat: add parser"}]}}"#,
                r#"{"type":"result","subtype":"success","is_error":false,"result":"feat: add parser"}"#,
            ],
        );
        assert_eq!(
            events,
            vec![
                CliEvent::Text("feat: add ".to_owned()),
                CliEvent::Text("parser".to_owned()),
            ]
        );
    }

    #[test]
    fn claude_falls_back_to_the_result_without_deltas() {
        let events = feed(
            CliTool::ClaudeCode,
            &[r#"{"type":"result","subtype":"success","is_error":false,"result":"fix: typo\n"}"#],
        );
        assert_eq!(events, vec![CliEvent::Text("fix: typo".to_owned())]);
    }

    #[test]
    fn claude_reports_error_results() {
        let events = feed(
            CliTool::ClaudeCode,
            &[
                "not json",
                r#"{"type":"result","subtype":"success","is_error":true,"result":"Not logged in"}"#,
            ],
        );
        assert_eq!(events, vec![CliEvent::Failed("Not logged in".to_owned())]);
        let events = feed(
            CliTool::ClaudeCode,
            &[
                r#"{"type":"result","subtype":"error_during_execution","is_error":true,"errors":["first","second"]}"#,
            ],
        );
        assert_eq!(events, vec![CliEvent::Failed("first; second".to_owned())]);
    }

    #[test]
    fn codex_emits_only_the_final_agent_message() {
        let events = feed(
            CliTool::Codex,
            &[
                r#"{"type":"thread.started","thread_id":"t1"}"#,
                r#"{"type":"turn.started"}"#,
                r#"{"type":"item.completed","item":{"id":"item_0","type":"reasoning","text":"thinking"}}"#,
                r#"{"type":"item.completed","item":{"id":"item_1","type":"agent_message","text":"draft"}}"#,
                r#"{"type":"item.completed","item":{"id":"item_2","type":"agent_message","text":"feat: add parser"}}"#,
                r#"{"type":"turn.completed","usage":{"input_tokens":1,"output_tokens":1}}"#,
            ],
        );
        assert_eq!(events, vec![CliEvent::Text("feat: add parser".to_owned())]);
    }

    #[test]
    fn codex_unwraps_turn_failures() {
        let events = feed(
            CliTool::Codex,
            &[
                r#"{"type":"item.completed","item":{"id":"item_0","type":"error","message":"warning"}}"#,
                r#"{"type":"error","message":"Reconnecting... 1/5"}"#,
                r#"{"type":"turn.failed","error":{"message":"{\"type\":\"error\",\"status\":400,\"error\":{\"type\":\"invalid_request_error\",\"message\":\"model unavailable\"}}"}}"#,
            ],
        );
        assert_eq!(
            events,
            vec![
                CliEvent::Warning("Reconnecting... 1/5".to_owned()),
                CliEvent::Failed("model unavailable".to_owned()),
            ]
        );
    }

    #[test]
    fn invocations_disable_tools_and_persistence() {
        let claude = args(CliTool::ClaudeCode, Path::new("/tmp/work"));
        for flag in ["--no-session-persistence", "--tools", "--strict-mcp-config"] {
            assert!(claude.iter().any(|arg| arg == flag), "{flag}");
        }
        let codex = args(CliTool::Codex, Path::new("/tmp/work"));
        for flag in ["--ephemeral", "read-only", "--ignore-user-config"] {
            assert!(codex.iter().any(|arg| arg == flag), "{flag}");
        }
        assert_eq!(codex.last().map(|arg| arg.as_os_str()), Some("-".as_ref()));
    }
}

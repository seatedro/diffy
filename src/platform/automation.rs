use std::fs;
use std::io::{self, Write};
use std::path::Path;

use serde::Serialize;

use crate::core::compare::{CompareMode, LayoutMode, RendererKind};
use crate::core::error::Result;
use crate::platform::persistence::Settings;
use crate::ui::state::{AppState, AsyncStatus, Screen, ToastKind};

#[derive(Debug, Clone, Serialize)]
pub struct StateDump {
    pub current_screen: &'static str,
    pub repository_status: &'static str,
    pub compare_status: &'static str,
    pub compare: CompareDump,
    pub selected_file_index: Option<usize>,
    pub selected_file_path: Option<String>,
    pub viewport: ViewportDump,
    pub pull_request: PullRequestDump,
    pub auth: AuthDump,
    pub last_error: Option<String>,
    pub toasts: Vec<ToastDump>,
    pub settings: SettingsDump,
    pub debug: DebugDump,
}

#[derive(Debug, Clone, Serialize)]
pub struct CompareDump {
    pub repo_path: Option<String>,
    pub left_ref: String,
    pub right_ref: String,
    pub resolved_left: Option<String>,
    pub resolved_right: Option<String>,
    pub mode: CompareMode,
    pub layout: LayoutMode,
    pub renderer: RendererKind,
    pub compare_generation: u64,
    pub file_count: usize,
    pub used_fallback: bool,
    pub fallback_message: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ViewportDump {
    pub layout: LayoutMode,
    pub wrap_enabled: bool,
    pub wrap_column: u32,
    pub scroll_top_px: u32,
    pub content_height_px: u32,
    pub viewport_width_px: u32,
    pub viewport_height_px: u32,
    pub hovered_row: Option<usize>,
    pub visible_row_start: Option<usize>,
    pub visible_row_end: Option<usize>,
    pub focused: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct PullRequestDump {
    pub status: &'static str,
    pub url_input: String,
    pub title: Option<String>,
    pub number: Option<i32>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AuthDump {
    pub status: &'static str,
    pub token_present: bool,
    pub user_code: Option<String>,
    pub verification_uri: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SettingsDump {
    pub recent_repos: Vec<String>,
    pub theme_name: String,
    pub theme_mode: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct DebugDump {
    pub last_scene_primitive_count: usize,
    pub last_frame_time_us: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct FilesDump {
    pub selected_file_index: Option<usize>,
    pub selected_file_path: Option<String>,
    pub files: Vec<FileDump>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FileDump {
    pub path: String,
    pub status: String,
    pub additions: i32,
    pub deletions: i32,
    pub is_binary: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ErrorDump {
    pub last_error: Option<String>,
    pub toasts: Vec<ToastDump>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ToastDump {
    pub kind: &'static str,
    pub message: String,
}

impl From<&AppState> for StateDump {
    fn from(state: &AppState) -> Self {
        Self {
            current_screen: screen_name(state.current_screen),
            repository_status: async_status_name(state.repository.status),
            compare_status: async_status_name(state.workspace.status),
            compare: CompareDump {
                repo_path: state
                    .compare
                    .repo_path
                    .as_ref()
                    .map(|path| path.display().to_string()),
                left_ref: state.compare.left_ref.clone(),
                right_ref: state.compare.right_ref.clone(),
                resolved_left: state.compare.resolved_left.clone(),
                resolved_right: state.compare.resolved_right.clone(),
                mode: state.compare.mode,
                layout: state.compare.layout,
                renderer: state.compare.renderer,
                compare_generation: state.workspace.compare_generation,
                file_count: state.workspace.files.len(),
                used_fallback: state.workspace.used_fallback,
                fallback_message: state.workspace.fallback_message.clone(),
            },
            selected_file_index: state.workspace.selected_file_index,
            selected_file_path: state.workspace.selected_file_path.clone(),
            viewport: ViewportDump {
                layout: state.viewport.layout,
                wrap_enabled: state.viewport.wrap_enabled,
                wrap_column: state.viewport.wrap_column,
                scroll_top_px: state.viewport.scroll_top_px,
                content_height_px: state.viewport.content_height_px,
                viewport_width_px: state.viewport.viewport_width_px,
                viewport_height_px: state.viewport.viewport_height_px,
                hovered_row: state.viewport.hovered_row,
                visible_row_start: state.viewport.visible_row_start,
                visible_row_end: state.viewport.visible_row_end,
                focused: state.viewport.focused,
            },
            pull_request: PullRequestDump {
                status: async_status_name(state.github.pull_request.status),
                url_input: state.github.pull_request.url_input.clone(),
                title: state
                    .github
                    .pull_request
                    .info
                    .as_ref()
                    .map(|info| info.title.clone()),
                number: state
                    .github
                    .pull_request
                    .info
                    .as_ref()
                    .map(|info| info.number),
            },
            auth: AuthDump {
                status: async_status_name(state.github.auth.status),
                token_present: state.github.auth.token_present,
                user_code: state
                    .github
                    .auth
                    .device_flow
                    .as_ref()
                    .map(|state| state.user_code.clone()),
                verification_uri: state
                    .github
                    .auth
                    .device_flow
                    .as_ref()
                    .map(|state| state.verification_uri.clone()),
            },
            last_error: state.last_error.clone(),
            toasts: state
                .toasts
                .iter()
                .map(|toast| ToastDump {
                    kind: toast_kind_name(toast.kind),
                    message: toast.message.clone(),
                })
                .collect(),
            settings: SettingsDump::from(&state.settings),
            debug: DebugDump {
                last_scene_primitive_count: state.debug.last_scene_primitive_count,
                last_frame_time_us: state.debug.last_frame_time_us,
            },
        }
    }
}

impl From<&Settings> for SettingsDump {
    fn from(settings: &Settings) -> Self {
        Self {
            recent_repos: settings
                .recent_repos
                .iter()
                .map(|path| path.display().to_string())
                .collect(),
            theme_name: settings.theme_name.clone(),
            theme_mode: format!("{:?}", settings.theme_mode).to_ascii_lowercase(),
        }
    }
}

impl From<&AppState> for FilesDump {
    fn from(state: &AppState) -> Self {
        Self {
            selected_file_index: state.workspace.selected_file_index,
            selected_file_path: state.workspace.selected_file_path.clone(),
            files: state
                .workspace
                .files
                .iter()
                .map(|file| FileDump {
                    path: file.path.clone(),
                    status: file.status.clone(),
                    additions: file.additions,
                    deletions: file.deletions,
                    is_binary: file.is_binary,
                })
                .collect(),
        }
    }
}

impl From<&AppState> for ErrorDump {
    fn from(state: &AppState) -> Self {
        Self {
            last_error: state.last_error.clone(),
            toasts: state
                .toasts
                .iter()
                .map(|toast| ToastDump {
                    kind: toast_kind_name(toast.kind),
                    message: toast.message.clone(),
                })
                .collect(),
        }
    }
}

pub fn write_json<T>(path: &Path, value: &T) -> Result<()>
where
    T: Serialize,
{
    let bytes = serde_json::to_vec_pretty(value)?;
    if path == Path::new("-") {
        let mut stdout = io::stdout().lock();
        stdout.write_all(&bytes)?;
        stdout.write_all(b"\n")?;
        stdout.flush()?;
        return Ok(());
    }

    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, bytes)?;
    Ok(())
}

fn screen_name(screen: Screen) -> &'static str {
    match screen {
        Screen::Welcome => "welcome",
        Screen::Compare => "compare",
        Screen::Diff => "diff",
    }
}

fn async_status_name(status: AsyncStatus) -> &'static str {
    match status {
        AsyncStatus::Idle => "idle",
        AsyncStatus::Loading => "loading",
        AsyncStatus::Ready => "ready",
        AsyncStatus::Failed => "failed",
    }
}

fn toast_kind_name(kind: ToastKind) -> &'static str {
    match kind {
        ToastKind::Info => "info",
        ToastKind::Error => "error",
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::{FilesDump, StateDump};
    use crate::platform::persistence::Settings;
    use crate::platform::startup::{Args, StartupOptions};
    use crate::ui::state::AppState;

    #[test]
    fn state_dump_reflects_bootstrap_defaults() {
        let startup = StartupOptions::from_parts(
            Args::parse_from(["diffy"]),
            None,
            "client".to_owned(),
            false,
        );
        let (state, _) = AppState::bootstrap(startup, Settings::default());

        let dump = StateDump::from(&state);
        assert_eq!(dump.current_screen, "welcome");
        assert_eq!(
            dump.compare.layout,
            crate::core::compare::LayoutMode::Unified
        );
    }

    #[test]
    fn files_dump_is_empty_before_compare() {
        let startup = StartupOptions::from_parts(
            Args::parse_from(["diffy"]),
            None,
            "client".to_owned(),
            false,
        );
        let (state, _) = AppState::bootstrap(startup, Settings::default());

        let dump = FilesDump::from(&state);
        assert!(dump.files.is_empty());
    }
}

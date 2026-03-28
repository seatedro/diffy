use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::core::compare::{CompareMode, CompareOutput, CompareSpec, LayoutMode, RendererKind};
use crate::core::diff::FileDiff;
use crate::core::vcs::git::{BranchInfo, CommitInfo, TagInfo};
use crate::core::vcs::github::{DeviceFlowState, PullRequestInfo};
use crate::platform::persistence::{PersistedCompare, Settings};
use crate::platform::startup::StartupOptions;
use crate::ui::actions::Action;
use crate::ui::diff_viewport::render_doc::{RenderDoc, build_render_doc};
use crate::ui::diff_viewport::state::DiffViewportState;
use crate::ui::effects::{CompareRequest, Effect};
use crate::ui::events::{AppEvent, CompareFinished, RepositoryLoaded};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Screen {
    #[default]
    Welcome,
    Compare,
    Diff,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum AsyncStatus {
    #[default]
    Idle,
    Loading,
    Ready,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompareState {
    pub repo_path: Option<PathBuf>,
    pub left_ref: String,
    pub right_ref: String,
    pub mode: CompareMode,
    pub layout: LayoutMode,
    pub renderer: RendererKind,
    pub resolved_left: Option<String>,
    pub resolved_right: Option<String>,
}

impl Default for CompareState {
    fn default() -> Self {
        Self {
            repo_path: None,
            left_ref: String::new(),
            right_ref: String::new(),
            mode: CompareMode::default(),
            layout: LayoutMode::default(),
            renderer: RendererKind::default(),
            resolved_left: None,
            resolved_right: None,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RepositoryState {
    pub status: AsyncStatus,
    pub branches: Vec<BranchInfo>,
    pub tags: Vec<TagInfo>,
    pub commits: Vec<CommitInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileListEntry {
    pub path: String,
    pub status: String,
    pub additions: i32,
    pub deletions: i32,
    pub is_binary: bool,
}

#[derive(Debug, Clone)]
pub struct ActiveFile {
    pub index: usize,
    pub path: String,
    pub file: FileDiff,
    pub render_doc: RenderDoc,
}

#[derive(Debug, Clone, Default)]
pub struct WorkspaceState {
    pub status: AsyncStatus,
    pub compare_generation: u64,
    pub files: Vec<FileListEntry>,
    pub selected_file_index: Option<usize>,
    pub selected_file_path: Option<String>,
    pub compare_output: Option<CompareOutput>,
    pub active_file: Option<ActiveFile>,
    pub raw_diff_len: usize,
    pub used_fallback: bool,
    pub fallback_message: String,
}

impl WorkspaceState {
    fn clear_compare(&mut self) {
        self.status = AsyncStatus::Idle;
        self.files.clear();
        self.selected_file_index = None;
        self.selected_file_path = None;
        self.compare_output = None;
        self.active_file = None;
        self.raw_diff_len = 0;
        self.used_fallback = false;
        self.fallback_message.clear();
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PullRequestState {
    pub status: AsyncStatus,
    pub url_input: String,
    pub info: Option<PullRequestInfo>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GitHubAuthState {
    pub status: AsyncStatus,
    pub device_flow: Option<DeviceFlowState>,
    pub token_present: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GitHubState {
    pub client_id: String,
    pub auth: GitHubAuthState,
    pub pull_request: PullRequestState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastKind {
    Info,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Toast {
    pub kind: ToastKind,
    pub message: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StartupState {
    pub auto_compare_pending: bool,
    pub pending_pr_url: Option<String>,
    pub preferred_file_index: Option<usize>,
    pub preferred_file_path: Option<String>,
    pub hidden_window: bool,
    pub exit_after: Option<Duration>,
    pub dump_state_json: Option<PathBuf>,
    pub dump_files_json: Option<PathBuf>,
    pub dump_errors_json: Option<PathBuf>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DebugState {
    pub last_scene_primitive_count: usize,
    pub last_frame_time_us: u64,
}

#[derive(Debug, Clone)]
pub struct AppState {
    pub current_screen: Screen,
    pub compare: CompareState,
    pub repository: RepositoryState,
    pub workspace: WorkspaceState,
    pub viewport: DiffViewportState,
    pub github: GitHubState,
    pub settings: Settings,
    pub startup: StartupState,
    pub last_error: Option<String>,
    pub toasts: Vec<Toast>,
    pub debug: DebugState,
}

impl AppState {
    pub fn bootstrap(startup: StartupOptions, mut settings: Settings) -> (Self, Vec<Effect>) {
        if startup.github_token.is_some() {
            settings.github_token = startup.github_token.clone();
        }

        let persisted = matching_persisted_compare(&startup, &settings).cloned();
        let repo_path = startup.args.repo.clone();
        let left_ref = startup
            .args
            .left
            .clone()
            .or_else(|| persisted.as_ref().map(|compare| compare.left_ref.clone()))
            .unwrap_or_default();
        let right_ref = startup
            .args
            .right
            .clone()
            .or_else(|| persisted.as_ref().map(|compare| compare.right_ref.clone()))
            .unwrap_or_default();
        let mode = startup
            .args
            .compare_mode
            .or_else(|| persisted.as_ref().map(|compare| compare.mode))
            .unwrap_or_default();
        let layout = startup
            .args
            .layout
            .or_else(|| persisted.as_ref().map(|compare| compare.layout))
            .unwrap_or(settings.viewport.layout);
        let renderer = startup
            .args
            .renderer
            .or_else(|| persisted.as_ref().map(|compare| compare.renderer))
            .unwrap_or_default();
        let auto_compare_pending = startup.wants_compare(mode, &left_ref, &right_ref);
        let github_client_id = startup.github_client_id.clone();

        let mut state = Self {
            current_screen: if repo_path.is_some() {
                Screen::Compare
            } else {
                Screen::Welcome
            },
            compare: CompareState {
                repo_path: repo_path.clone(),
                left_ref,
                right_ref,
                mode,
                layout,
                renderer,
                resolved_left: None,
                resolved_right: None,
            },
            repository: RepositoryState::default(),
            workspace: WorkspaceState::default(),
            viewport: DiffViewportState {
                layout,
                wrap_enabled: settings.viewport.wrap_enabled,
                wrap_column: settings.viewport.wrap_column,
                ..DiffViewportState::default()
            },
            github: GitHubState {
                client_id: github_client_id,
                auth: GitHubAuthState {
                    token_present: settings.github_token.is_some(),
                    ..GitHubAuthState::default()
                },
                pull_request: PullRequestState {
                    url_input: startup.args.open_pr.clone().unwrap_or_default(),
                    ..PullRequestState::default()
                },
            },
            settings,
            startup: StartupState {
                auto_compare_pending,
                pending_pr_url: startup.args.open_pr.clone(),
                preferred_file_index: startup.args.file_index,
                preferred_file_path: startup.args.file_path.clone(),
                hidden_window: startup.hidden_window(),
                exit_after: startup.exit_after(),
                dump_state_json: startup.args.dump_state_json.clone(),
                dump_files_json: startup.args.dump_files_json.clone(),
                dump_errors_json: startup.args.dump_errors_json.clone(),
            },
            last_error: None,
            toasts: Vec::new(),
            debug: DebugState::default(),
        };
        state.sync_settings_snapshot();

        let mut effects = Vec::new();
        if let Some(path) = repo_path {
            state.repository.status = AsyncStatus::Loading;
            effects.push(Effect::LoadRepository { path });
        }
        (state, effects)
    }

    pub fn apply_action(&mut self, action: Action) -> Vec<Effect> {
        match action {
            Action::Bootstrap => Vec::new(),
            Action::OpenRepository(path) => {
                self.current_screen = Screen::Compare;
                self.compare.repo_path = Some(path.clone());
                self.compare.resolved_left = None;
                self.compare.resolved_right = None;
                self.repository.status = AsyncStatus::Loading;
                self.workspace.clear_compare();
                self.last_error = None;
                self.github.pull_request.info = None;
                self.sync_settings_snapshot();
                vec![
                    Effect::SaveSettings(self.settings.clone()),
                    Effect::LoadRepository { path },
                ]
            }
            Action::SetLeftRef(value) => {
                self.compare.left_ref = value;
                self.compare.resolved_left = None;
                self.persist_settings_effect()
            }
            Action::SetRightRef(value) => {
                self.compare.right_ref = value;
                self.compare.resolved_right = None;
                self.persist_settings_effect()
            }
            Action::SetCompareMode(mode) => {
                self.compare.mode = mode;
                self.persist_settings_effect()
            }
            Action::SetLayoutMode(layout) => {
                self.compare.layout = layout;
                self.viewport.layout = layout;
                self.persist_settings_effect()
            }
            Action::SetRenderer(renderer) => {
                self.compare.renderer = renderer;
                self.persist_settings_effect()
            }
            Action::StartCompare => self.kickoff_compare(),
            Action::SelectFile(index) => {
                self.select_loaded_file(index);
                Vec::new()
            }
            Action::SelectFilePath(path) => {
                if let Some(index) = self
                    .workspace
                    .files
                    .iter()
                    .position(|file| file.path == path)
                {
                    self.select_loaded_file(index);
                } else {
                    self.startup.preferred_file_path = Some(path);
                }
                Vec::new()
            }
            Action::OpenPullRequest(url) => {
                self.github.pull_request.url_input = url.clone();
                let Some(repo_path) = self.compare.repo_path.clone() else {
                    self.push_error("Open a repository before loading a pull request.");
                    return Vec::new();
                };
                self.github.pull_request.status = AsyncStatus::Loading;
                vec![Effect::LoadPullRequest {
                    url,
                    repo_path,
                    github_token: self.settings.github_token.clone(),
                }]
            }
            Action::StartGitHubDeviceFlow => {
                self.github.auth.status = AsyncStatus::Loading;
                vec![Effect::StartDeviceFlow {
                    client_id: self.github.client_id.clone(),
                }]
            }
            Action::DismissToast(index) => {
                if index < self.toasts.len() {
                    self.toasts.remove(index);
                }
                Vec::new()
            }
            Action::ToggleWrap => {
                self.viewport.wrap_enabled = !self.viewport.wrap_enabled;
                self.persist_settings_effect()
            }
            Action::SetWrapColumn(column) => {
                self.viewport.wrap_column = column;
                self.persist_settings_effect()
            }
        }
    }

    pub fn apply_event(&mut self, event: AppEvent) -> Vec<Effect> {
        match event {
            AppEvent::RepositoryLoaded(payload) => self.handle_repository_loaded(payload),
            AppEvent::RepositoryLoadFailed { path, message } => {
                if self.compare.repo_path.as_ref() == Some(&path) {
                    self.repository.status = AsyncStatus::Failed;
                    self.push_error(&message);
                }
                Vec::new()
            }
            AppEvent::CompareFinished(payload) => self.handle_compare_finished(payload),
            AppEvent::CompareFailed {
                generation,
                message,
            } => {
                if generation == self.workspace.compare_generation {
                    self.workspace.status = AsyncStatus::Failed;
                    self.current_screen = Screen::Compare;
                    self.push_error(&message);
                }
                Vec::new()
            }
            AppEvent::PullRequestLoaded {
                url,
                info,
                left_ref,
                right_ref,
            } => {
                self.github.pull_request.status = AsyncStatus::Ready;
                self.github.pull_request.url_input = url;
                self.github.pull_request.info = Some(info);
                self.compare.left_ref = left_ref;
                self.compare.right_ref = right_ref;
                self.compare.mode = CompareMode::ThreeDot;
                self.kickoff_compare()
            }
            AppEvent::PullRequestLoadFailed { message, .. } => {
                self.github.pull_request.status = AsyncStatus::Failed;
                self.push_error(&message);
                Vec::new()
            }
            AppEvent::DeviceFlowStarted(device_flow) => {
                self.github.auth.status = AsyncStatus::Loading;
                self.github.auth.device_flow = Some(device_flow.clone());
                vec![
                    Effect::OpenBrowser {
                        url: device_flow.verification_uri.clone(),
                    },
                    Effect::PollDeviceFlow {
                        client_id: self.github.client_id.clone(),
                        device_code: device_flow.device_code,
                        interval_seconds: device_flow.interval,
                    },
                ]
            }
            AppEvent::DeviceFlowStartFailed { message } => {
                self.github.auth.status = AsyncStatus::Failed;
                self.push_error(&message);
                Vec::new()
            }
            AppEvent::DeviceFlowCompleted { token } => {
                self.github.auth.status = AsyncStatus::Ready;
                self.github.auth.device_flow = None;
                self.github.auth.token_present = true;
                self.settings.github_token = Some(token);
                self.push_info("GitHub authentication completed.");
                self.persist_settings_effect()
            }
            AppEvent::DeviceFlowFailed { message } => {
                self.github.auth.status = AsyncStatus::Failed;
                self.push_error(&message);
                Vec::new()
            }
            AppEvent::SettingsSaved => Vec::new(),
            AppEvent::SettingsSaveFailed { message } => {
                self.push_error(&message);
                Vec::new()
            }
            AppEvent::BrowserOpenFailed { message } => {
                self.push_error(&message);
                Vec::new()
            }
        }
    }

    pub fn window_title(&self) -> String {
        let screen = match self.current_screen {
            Screen::Welcome => "welcome",
            Screen::Compare => "compare",
            Screen::Diff => "diff",
        };
        let repo = self
            .compare
            .repo_path
            .as_deref()
            .and_then(Path::file_name)
            .and_then(|value| value.to_str())
            .unwrap_or("native");
        if let Some(path) = self.workspace.selected_file_path.as_deref() {
            format!("diffy native - {repo} [{screen}] {path}")
        } else {
            format!("diffy native - {repo} [{screen}]")
        }
    }

    fn handle_repository_loaded(&mut self, payload: RepositoryLoaded) -> Vec<Effect> {
        if self.compare.repo_path.as_ref() != Some(&payload.path) {
            return Vec::new();
        }

        self.repository.status = AsyncStatus::Ready;
        self.repository.branches = payload.branches;
        self.repository.tags = payload.tags;
        self.repository.commits = payload.commits;
        self.settings.remember_repo(&payload.path);

        let mut effects = self.persist_settings_effect();
        if let Some(url) = self.startup.pending_pr_url.clone() {
            self.startup.pending_pr_url = None;
            self.github.pull_request.status = AsyncStatus::Loading;
            effects.push(Effect::LoadPullRequest {
                url,
                repo_path: payload.path,
                github_token: self.settings.github_token.clone(),
            });
        } else if self.startup.auto_compare_pending {
            self.startup.auto_compare_pending = false;
            effects.extend(self.kickoff_compare());
        }
        effects
    }

    fn handle_compare_finished(&mut self, payload: CompareFinished) -> Vec<Effect> {
        if payload.generation != self.workspace.compare_generation {
            return Vec::new();
        }

        self.workspace.status = AsyncStatus::Ready;
        self.current_screen = Screen::Diff;
        self.compare.layout = payload.spec.layout;
        self.compare.renderer = payload.spec.renderer;
        self.compare.resolved_left = Some(payload.resolved_left);
        self.compare.resolved_right = Some(payload.resolved_right);
        self.workspace.raw_diff_len = payload.output.raw_diff.len();
        self.workspace.used_fallback = payload.output.used_fallback;
        self.workspace.fallback_message = payload.output.fallback_message.clone();
        self.workspace.files = build_file_entries(&payload.output.files);
        self.workspace.compare_output = Some(payload.output);

        let preferred_index = self
            .startup
            .preferred_file_index
            .or(self.workspace.selected_file_index);
        let preferred_path = self
            .startup
            .preferred_file_path
            .clone()
            .or_else(|| self.workspace.selected_file_path.clone());

        if let Some(index) = preferred_path
            .as_deref()
            .and_then(|path| {
                self.workspace
                    .files
                    .iter()
                    .position(|file| file.path == path)
            })
            .or(preferred_index.filter(|index| *index < self.workspace.files.len()))
            .or_else(|| (!self.workspace.files.is_empty()).then_some(0))
        {
            self.select_loaded_file(index);
        } else {
            self.workspace.selected_file_index = None;
            self.workspace.selected_file_path = None;
            self.workspace.active_file = None;
        }

        if self.workspace.used_fallback && !self.workspace.fallback_message.is_empty() {
            self.push_info(&self.workspace.fallback_message.clone());
        }
        Vec::new()
    }

    fn kickoff_compare(&mut self) -> Vec<Effect> {
        let Some(repo_path) = self.compare.repo_path.clone() else {
            self.push_error("Open a repository before starting a compare.");
            return Vec::new();
        };

        if !compare_refs_are_valid(
            self.compare.mode,
            &self.compare.left_ref,
            &self.compare.right_ref,
        ) {
            self.push_error("Provide the required refs for the selected compare mode.");
            return Vec::new();
        }

        self.current_screen = Screen::Compare;
        self.workspace.status = AsyncStatus::Loading;
        self.workspace.compare_generation = self.workspace.compare_generation.saturating_add(1);
        self.sync_settings_snapshot();

        vec![
            Effect::SaveSettings(self.settings.clone()),
            Effect::RunCompare {
                generation: self.workspace.compare_generation,
                request: CompareRequest {
                    repo_path,
                    spec: CompareSpec {
                        mode: self.compare.mode,
                        left_ref: self.compare.left_ref.clone(),
                        right_ref: self.compare.right_ref.clone(),
                        renderer: self.compare.renderer,
                        layout: self.compare.layout,
                    },
                    github_token: self.settings.github_token.clone(),
                },
            },
        ]
    }

    fn persist_settings_effect(&mut self) -> Vec<Effect> {
        self.sync_settings_snapshot();
        vec![Effect::SaveSettings(self.settings.clone())]
    }

    fn sync_settings_snapshot(&mut self) {
        self.settings.viewport.wrap_enabled = self.viewport.wrap_enabled;
        self.settings.viewport.wrap_column = self.viewport.wrap_column;
        self.settings.viewport.layout = self.compare.layout;
        self.settings.last_compare = Some(PersistedCompare {
            repo_path: self.compare.repo_path.clone(),
            left_ref: self.compare.left_ref.clone(),
            right_ref: self.compare.right_ref.clone(),
            mode: self.compare.mode,
            layout: self.compare.layout,
            renderer: self.compare.renderer,
        });
    }

    fn select_loaded_file(&mut self, index: usize) {
        let Some(output) = self.workspace.compare_output.as_ref() else {
            self.startup.preferred_file_index = Some(index);
            return;
        };
        let Some(file) = output.files.get(index) else {
            self.push_error("Selected file index is out of range.");
            return;
        };

        self.workspace.selected_file_index = Some(index);
        self.workspace.selected_file_path = Some(file.path.clone());
        self.workspace.active_file = Some(ActiveFile {
            index,
            path: file.path.clone(),
            file: file.clone(),
            render_doc: build_render_doc(file, index, &output.text_buffer, &output.token_buffer),
        });
    }

    fn push_error(&mut self, message: &str) {
        self.last_error = Some(message.to_owned());
        self.push_toast(ToastKind::Error, message);
    }

    fn push_info(&mut self, message: &str) {
        self.push_toast(ToastKind::Info, message);
    }

    fn push_toast(&mut self, kind: ToastKind, message: &str) {
        self.toasts.push(Toast {
            kind,
            message: message.to_owned(),
        });
        if self.toasts.len() > 8 {
            self.toasts.remove(0);
        }
    }
}

fn matching_persisted_compare<'a>(
    startup: &'a StartupOptions,
    settings: &'a Settings,
) -> Option<&'a PersistedCompare> {
    settings.last_compare.as_ref().filter(|compare| {
        startup.args.repo.is_some() && compare.repo_path.as_ref() == startup.args.repo.as_ref()
    })
}

fn compare_refs_are_valid(mode: CompareMode, left_ref: &str, right_ref: &str) -> bool {
    match mode {
        CompareMode::SingleCommit => !left_ref.is_empty() || !right_ref.is_empty(),
        CompareMode::TwoDot | CompareMode::ThreeDot => {
            !left_ref.is_empty() && !right_ref.is_empty()
        }
    }
}

fn build_file_entries(files: &[FileDiff]) -> Vec<FileListEntry> {
    files.iter().map(FileListEntry::from).collect()
}

impl From<&FileDiff> for FileListEntry {
    fn from(value: &FileDiff) -> Self {
        Self {
            path: value.path.clone(),
            status: value.status.clone(),
            additions: value.additions,
            deletions: value.deletions,
            is_binary: value.is_binary,
        }
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::{AppState, Screen};
    use crate::core::compare::{CompareMode, CompareOutput, LayoutMode, RendererKind};
    use crate::core::diff::FileDiff;
    use crate::platform::persistence::{Settings, SettingsStore};
    use crate::platform::startup::{Args, StartupOptions};
    use crate::ui::actions::Action;

    #[test]
    fn bootstrap_queues_repo_load_for_startup_repo() {
        let startup = StartupOptions::from_parts(
            Args {
                repo: Some("C:\\repo".into()),
                left: Some("main".to_owned()),
                right: Some("feature".to_owned()),
                compare_mode: Some(CompareMode::ThreeDot),
                layout: Some(LayoutMode::Split),
                renderer: Some(RendererKind::Builtin),
                file_index: None,
                file_path: None,
                open_pr: None,
                exit_after_ms: None,
                hidden_window: true,
                dump_state_json: None,
                dump_files_json: None,
                dump_errors_json: None,
            },
            None,
            "client".to_owned(),
            false,
        );

        let (state, effects) = AppState::bootstrap(startup, Settings::default());
        assert_eq!(state.current_screen, Screen::Compare);
        assert_eq!(state.compare.left_ref, "main");
        assert_eq!(effects.len(), 1);
    }

    #[test]
    fn selecting_file_without_compare_sets_preference() {
        let startup = StartupOptions::from_parts(
            Args::parse_from(["diffy"]),
            None,
            "client".to_owned(),
            false,
        );
        let (mut state, _) = AppState::bootstrap(startup, Settings::default());
        state.apply_action(Action::SelectFilePath("src/main.rs".to_owned()));
        assert_eq!(
            state.startup.preferred_file_path.as_deref(),
            Some("src/main.rs")
        );
    }

    #[test]
    fn compare_start_requires_refs() {
        let startup = StartupOptions::from_parts(
            Args::parse_from(["diffy"]),
            None,
            "client".to_owned(),
            false,
        );
        let (mut state, _) = AppState::bootstrap(startup, Settings::default());
        state.compare.repo_path = Some("C:\\repo".into());
        let effects = state.apply_action(Action::StartCompare);
        assert!(effects.is_empty());
        assert!(state.last_error.is_some());
    }

    #[test]
    fn selecting_loaded_file_updates_path() {
        let startup = StartupOptions::from_parts(
            Args::parse_from(["diffy"]),
            None,
            "client".to_owned(),
            false,
        );
        let (mut state, _) = AppState::bootstrap(startup, Settings::default());
        state.workspace.compare_output = Some(CompareOutput {
            files: vec![FileDiff {
                path: "src/main.rs".to_owned(),
                ..FileDiff::default()
            }],
            ..CompareOutput::default()
        });
        let file = FileDiff {
            path: "src/main.rs".to_owned(),
            ..FileDiff::default()
        };
        state.workspace.files = vec![(&file).into()];

        state.apply_action(Action::SelectFile(0));
        assert_eq!(
            state.workspace.selected_file_path.as_deref(),
            Some("src/main.rs")
        );
    }

    #[test]
    fn settings_store_type_is_constructible_for_state_tests() {
        let _ = SettingsStore::new_default();
    }
}

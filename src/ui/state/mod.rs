mod ai;
mod app;
mod compare;
mod editor;
mod file_list;
mod github;
mod overlay;
mod presentation;
mod repository;
mod settings;
mod syntax;
mod text_compare;
mod text_edit;
mod ui;
mod update;
mod working_set;
mod workspace;

pub use self::app::*;
pub use self::compare::*;
pub use self::file_list::*;
pub use self::github::*;
pub use self::overlay::*;
pub use self::presentation::*;
pub use self::repository::*;
use self::syntax::*;
pub use self::text_compare::*;
pub use self::text_edit::*;
pub use self::ui::*;
pub use self::update::*;
pub use self::working_set::*;
pub use self::workspace::*;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use halogen::Store;
use halogen::reactive::{Signal, SignalStore};

use self::syntax::SyntaxRequestTracker;
use self::working_set::{FileWorkingSet, WorkingSetFileKey};
use crate::actions::Action;
use crate::core::compare::{
    CompareFileSummary, CompareMode, CompareOutput, ComparePath, LayoutMode, RendererKind,
};
use crate::core::forge::github::{
    CreatePullRequestReviewComment, DeviceFlowState, GitHubReviewSide, GitHubUser, PullRequestInfo,
    PullRequestReviewComment,
};
use crate::core::frecency::FrecencyStore;
use crate::core::review::{ReviewSession, ReviewSessionStatus, ReviewTarget};
use crate::core::syntax::Highlighter;
use crate::core::syntax::annotator::{SyntaxLineTokens, SyntaxRowWindow};
use crate::core::text::TokenBuffer;
use crate::core::update::{AvailableUpdate, StagedUpdate};
use crate::core::vcs::model::{
    ChangeBucket, FileChange, FileChangeStatus, FileOperation, JjOperation, PublishAction,
    PublishPlan, RefKind, RepoCapabilities, RepoLocation, VCS_PROFILE_GIT, VCS_PROFILE_JJ,
    VcsChange, VcsCompareRequest, VcsCompareSpec, VcsOperation, VcsOperationLogEntry, VcsRef,
};
use crate::core::vcs::patch;
use crate::editor::diff::render_doc::{
    CarbonStyleOverlays, RenderDoc, RenderLine, RenderRowKind, build_placeholder_render_doc,
    build_render_doc_from_carbon,
};
use crate::editor::diff::state::{EditorState, EditorStateStore, SearchMatch};
use crate::editor::{Editor, EditorMode};
use crate::effects::{
    AiEffect, BatchFileOperationRequest, CommitRequest, CompareEffect, CompareFileRequest,
    CompareFileStatsItem, CompareFileStatsRequest, CompareHistoryRequest, CompareRequest,
    CompareStatsRequest, CompareWorkPriority, Effect, FetchRemoteRequest, FileOperationRequest,
    GitHubEffect, PatchOperationRequest, PublishPlanRequest, PublishRequest, PullFfRequest,
    PushRequest, RepositoryEffect, SettingsEffect, StatusDiffRequest, SyntaxEffect, Task,
    TextCompareRequest, UiEffect, UpdateEffect, VcsOperationRequest,
};
use crate::events::{
    AppEvent, CompareFileFinished, CompareFileStat, CompareFileStatsReady, CompareFinished,
    CompareHistoryReady, CompareStatsReady, FileSyntaxReady, RepositoryChangeKind,
    RepositorySnapshot, RepositorySyncReason, StatusDiffFinished, TextCompareFinished,
};
use crate::fonts::{FontFamilyEntry, FontRole};
use crate::platform::persistence::{PersistedCompare, Settings};
use crate::platform::secrets::AiKeyKind;
use crate::platform::startup::{GitHubTokenStore, StartupOptions};
use crate::ui::components::ContextMenuState;
use crate::ui::design::{Sp, Sz};
use crate::ui::icons::lucide;
use crate::ui::theme::ThemeMode;
use crate::ui::virtual_list::{build_sectioned_rows, step_selection};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum AsyncStatus {
    #[default]
    Idle,
    Loading,
    Ready,
    Failed,
}

// App-chrome signals (focus, toasts, view routing, ...) live in the
// derived `UiStateStore` at `AppState::ui`.

#[derive(Debug)]
pub struct AppState {
    pub ui: UiStateStore,
    pub compare: CompareStateStore,
    pub repository: RepositoryStateStore,
    pub workspace: WorkspaceStateStore,
    pub file_list: FileListStateStore,
    pub overlays: OverlayStackStateStore,
    pub text_edit: TextEditStateStore,
    pub editor: EditorStateStore,
    pub github: GitHubStateStore,
    pub settings: Settings,
    pub startup: StartupState,
    pub context_menu: ContextMenuState,
    /// Memoized: `true` when `ui.focus` targets a text-editing field.
    pub text_focused: Signal<bool>,
    pub animation: crate::ui::animation::AnimationState,
    pub commit_editor: Editor,
    pub review_comment_editor: Editor,
    pub steering_prompt_editor: Editor,
    pub text_compare: TextCompareState,
    pub ai_openai_key: String,
    pub ai_anthropic_key: String,
    pub ai_openai_editing: bool,
    pub ai_anthropic_editing: bool,
    pub ai_generation_id: u64,
    pub ai_generation_active: bool,
    pub ai_generation_error: Option<String>,
    /// Shared reactive store. Signals (like `ui.sidebar_visible`) are handles
    /// into this store. Kept in `AppState` so state methods (apply_action etc.)
    /// can freely read/write signals without threading a store parameter.
    pub store: Rc<SignalStore>,
    pub debug: DebugStateStore,
    pub clock_ms: u64,
    pub next_toast_id: u64,
    pub frecency: Option<FrecencyStore>,
    pub theme_names: Vec<String>,
    pub theme_variants: Vec<crate::core::themes::ThemeVariant>,
    pub github_access_token: Option<String>,
    viewport_document_cache: Option<ViewportDocumentCache>,
    virtual_diff_document: VirtualDiffDocument,
    virtual_scroll: VirtualScrollModel,
    file_working_set: FileWorkingSet,
    syntax_requests: SyntaxRequestTracker,
    last_virtual_scroll_top_px: Option<u32>,
}

impl Default for AppState {
    fn default() -> Self {
        let store = Rc::new(SignalStore::default());
        let ui = UiStateStore::new_default(&store);
        let focus = ui.focus;
        let text_focused =
            store.create_memo(move |s| s.read(focus).is_some_and(|t| t.is_text_field()));
        let debug = DebugStateStore::new(&store, DebugState::default());
        let file_list = FileListStateStore::new_default(&store);
        let editor = EditorStateStore::new_default(&store);
        let overlays = OverlayStackStateStore::new_default(&store);
        let compare = CompareStateStore::new_default(&store);
        let repository = RepositoryStateStore::new_default(&store);
        let workspace = WorkspaceStateStore::new_default(&store);
        let text_edit = TextEditStateStore::new_default(&store);
        let github = GitHubStateStore::new_default(&store);
        Self {
            ui,
            compare,
            repository,
            workspace,
            file_list,
            overlays,
            text_edit,
            editor,
            github,
            settings: Settings::default(),
            startup: StartupState::default(),
            context_menu: ContextMenuState::default(),
            text_focused,
            animation: crate::ui::animation::AnimationState::default(),
            commit_editor: Editor::default(),
            review_comment_editor: Editor::default(),
            steering_prompt_editor: Editor::default(),
            text_compare: TextCompareState::default(),
            ai_openai_key: String::new(),
            ai_anthropic_key: String::new(),
            ai_openai_editing: false,
            ai_anthropic_editing: false,
            ai_generation_id: 0,
            ai_generation_active: false,
            ai_generation_error: None,
            debug,
            store,
            clock_ms: 0,
            next_toast_id: 1,
            frecency: None,
            theme_names: Vec::new(),
            theme_variants: Vec::new(),
            github_access_token: None,
            viewport_document_cache: None,
            virtual_diff_document: VirtualDiffDocument::default(),
            virtual_scroll: VirtualScrollModel::default(),
            file_working_set: FileWorkingSet::default(),
            syntax_requests: SyntaxRequestTracker::default(),
            last_virtual_scroll_top_px: None,
        }
    }
}

impl AppState {
    pub fn bootstrap(startup: StartupOptions, settings: Settings) -> (Self, Vec<Effect>) {
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
        let bootstrap_compare_started = repo_path.is_some()
            && startup.args.open_pr.is_none()
            && auto_compare_pending
            && (startup.args.left.is_some()
                || startup.args.right.is_some()
                || startup.args.compare_mode.is_some());

        let store = Rc::new(SignalStore::default());
        let ui = UiStateStore::new(
            &store,
            UiState {
                focus: Some(if repo_path.is_some() {
                    FocusTarget::TitleBar
                } else {
                    FocusTarget::WorkspacePrimaryButton
                }),
                ..UiState::default()
            },
        );
        let focus = ui.focus;
        let text_focused =
            store.create_memo(move |s| s.read(focus).is_some_and(|t| t.is_text_field()));
        let debug = DebugStateStore::new(&store, DebugState::default());
        let file_list = FileListStateStore::new_default(&store);
        let editor = EditorStateStore::new(
            &store,
            EditorState {
                layout,
                wrap_enabled: settings.viewport.wrap_enabled,
                wrap_column: settings.viewport.wrap_column,
                ..EditorState::default()
            },
        );
        let overlays = OverlayStackStateStore::new_default(&store);
        let compare = CompareStateStore::new(
            &store,
            CompareState {
                repo_path: repo_path.clone(),
                left_ref,
                right_ref,
                mode,
                layout,
                renderer,
                resolved_left: None,
                resolved_right: None,
            },
        );
        let repository = RepositoryStateStore::new_default(&store);
        let workspace = WorkspaceStateStore::new(
            &store,
            WorkspaceState {
                mode: if repo_path.is_some() && auto_compare_pending {
                    WorkspaceMode::Loading
                } else {
                    WorkspaceMode::Empty
                },
                ..WorkspaceState::default()
            },
        );
        let text_edit = TextEditStateStore::new_default(&store);
        let initial_token_present = settings.github_user.is_some();
        let github = GitHubStateStore::new(
            &store,
            GitHubState {
                client_id: startup.github_client_id.clone(),
                auth: GitHubAuthState {
                    token_present: initial_token_present,
                    user: settings.github_user.clone(),
                    ..GitHubAuthState::default()
                },
                pull_request: PullRequestState::default(),
            },
        );
        let mut state = Self {
            ui,
            compare,
            repository,
            workspace,
            file_list,
            overlays,
            text_edit,
            editor,
            github,
            settings,
            startup: StartupState {
                keyring_enabled: startup.keyring_enabled,
                github_token_store: startup.github_token_store,
                auto_compare_pending: auto_compare_pending && !bootstrap_compare_started,
                bootstrap_compare_started,
                pending_pr_url: startup.args.open_pr.clone(),
                preferred_file_index: startup.args.file_index,
                preferred_file_path: startup.args.file_path.clone(),
            },
            context_menu: ContextMenuState::default(),
            text_focused,
            animation: crate::ui::animation::AnimationState::default(),
            commit_editor: Editor::default(),
            review_comment_editor: Editor::default(),
            steering_prompt_editor: Editor::default(),
            text_compare: TextCompareState::default(),
            ai_openai_key: String::new(),
            ai_anthropic_key: String::new(),
            ai_openai_editing: false,
            ai_anthropic_editing: false,
            ai_generation_id: 0,
            ai_generation_active: false,
            ai_generation_error: None,
            debug,
            store,
            clock_ms: 0,
            next_toast_id: 1,
            frecency: crate::core::frecency::open_default_store(),
            theme_names: Vec::new(),
            theme_variants: Vec::new(),
            github_access_token: None,
            viewport_document_cache: None,
            virtual_diff_document: VirtualDiffDocument::default(),
            virtual_scroll: VirtualScrollModel::default(),
            file_working_set: FileWorkingSet::default(),
            syntax_requests: SyntaxRequestTracker::default(),
            last_virtual_scroll_top_px: None,
        };
        let seed_prompt = if state.settings.ai_steering_prompt.trim().is_empty() {
            crate::ai::DEFAULT_STEERING_PROMPT
        } else {
            state.settings.ai_steering_prompt.as_str()
        };
        state.steering_prompt_editor.set_text(seed_prompt);
        state.sync_settings_snapshot();

        let mut effects = Vec::new();
        if let Some(path) = repo_path {
            state
                .repository
                .status
                .set(&state.store, AsyncStatus::Loading);

            // Bootstrap: seed the loading panel so a slow cold-boot open
            // shows staged progress. Reveal is gated by the same 500ms
            // threshold as user-initiated opens — if the whole bootstrap
            // open completes within the threshold the panel never appears
            // and the user lands straight in the ready UI.
            let boot_gen = state
                .workspace
                .compare_generation
                .get(&state.store)
                .saturating_add(1);
            state
                .workspace
                .compare_generation
                .set(&state.store, boot_gen);
            effects.push(state.invalidate_syntax_epoch_effect());
            let repo_name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("repository")
                .to_owned();
            state.workspace.compare_progress.set(
                &state.store,
                Some(Arc::new(CompareProgress {
                    generation: boot_gen,
                    phase: ComparePhase::OpeningRepo,
                    subject: if bootstrap_compare_started {
                        LoadingSubject::Compare {
                            left_label: state.vcs_ui_profile().compare_ref_display_label(
                                &state.compare.left_ref.get(&state.store),
                            ),
                            right_label: state.vcs_ui_profile().compare_ref_display_label(
                                &state.compare.right_ref.get(&state.store),
                            ),
                        }
                    } else {
                        LoadingSubject::RepoOpen { name: repo_name }
                    },
                    started_at_ms: 0,
                    reveal_at_ms: COMPARE_REVEAL_DELAY_MS,
                    file_count_total: None,
                    files_loaded: 0,
                })),
            );

            effects.push(
                RepositoryEffect::SyncRepository {
                    path: path.clone(),
                    reason: RepositorySyncReason::Open,
                    reporter_generation: (!bootstrap_compare_started).then_some(boot_gen),
                }
                .into(),
            );
            effects.push(RepositoryEffect::WatchRepository { path: Some(path) }.into());
            if bootstrap_compare_started {
                effects.push(
                    CompareEffect::Run(Task {
                        generation: boot_gen,
                        request: CompareRequest {
                            repo_path: state.compare.repo_path.get(&state.store).unwrap(),
                            request: vcs_compare_request(
                                state.compare.mode.get(&state.store),
                                state.compare.left_ref.get(&state.store),
                                state.compare.right_ref.get(&state.store),
                                state.compare.layout.get(&state.store),
                                state.compare.renderer.get(&state.store),
                            ),
                            github_token: startup.github_token.clone(),
                        },
                    })
                    .into(),
                );
            }
        }
        if let Some(token) = startup.github_token.clone() {
            state.github_access_token = Some(token.clone());
            state.github.auth.token_present.set(&state.store, true);
            if startup.github_token_store.is_enabled() {
                effects.push(GitHubEffect::SaveGitHubToken(token).into());
            }
        } else if startup.github_token_store.is_enabled() {
            effects.push(GitHubEffect::LoadGitHubToken.into());
        }

        // Show the cached user + avatar optimistically while the token loads.
        if let Some(user) = state.settings.github_user.as_ref()
            && let Some(url) = avatar_url_sized(&user.avatar_url, 128)
        {
            state.github.auth.avatar_fetching.set(&state.store, true);
            effects.push(GitHubEffect::FetchAvatar { url }.into());
        }

        effects.push(SyntaxEffect::InstallCommonSyntaxPacks.into());
        if startup.keyring_enabled {
            effects.push(AiEffect::LoadAiKeys.into());
        }
        if state.update_polling_enabled() {
            effects.push(UpdateEffect::CheckForUpdates { silent: true }.into());
        }
        (state, effects)
    }

    pub fn apply_action<A: Into<Action>>(&mut self, action: A) -> Vec<Effect> {
        let action = action.into();
        match action {
            Action::App(action) => app::reduce_action(self, action),
            Action::Workspace(action) => workspace::reduce_action(self, action),
            Action::TextCompare(action) => text_compare::reduce_action(self, action),
            Action::Compare(action) => compare::reduce_action(self, action),
            Action::Repository(action) => repository::reduce_action(self, action),
            Action::FileList(action) => file_list::reduce_action(self, action),
            Action::Overlay(action) => overlay::reduce_action(self, action),
            Action::Editor(action) => editor::reduce_action(self, action),
            Action::TextEdit(action) => text_edit::reduce_action(self, action),
            Action::Settings(action) => settings::reduce_action(self, action),
            Action::GitHub(action) => github::reduce_action(self, action),
            Action::Update(action) => update::reduce_action(self, action),
            Action::Window(_) => Vec::new(),
            Action::Syntax(action) => syntax::reduce_action(self, action),
            Action::Ai(action) => ai::reduce_action(self, action),
            Action::Noop => Vec::new(),
        }
    }

    pub fn apply_event(&mut self, event: AppEvent) -> Vec<Effect> {
        match event {
            AppEvent::Ui(event) => app::reduce_event(self, event),
            AppEvent::Repository(event) => repository::reduce_event(self, event),
            AppEvent::Compare(event) => compare::reduce_event(self, event),
            AppEvent::GitHub(event) => github::reduce_event(self, event),
            AppEvent::Settings(event) => settings::reduce_event(self, event),
            AppEvent::Update(event) => update::reduce_event(self, event),
            AppEvent::Syntax(event) => syntax::reduce_event(self, event),
            AppEvent::Ai(event) => ai::reduce_event(self, event),
        }
    }
}

#[cfg(test)]
mod tests;

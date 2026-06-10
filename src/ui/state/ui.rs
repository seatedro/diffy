//! App-chrome state: view routing, settings sections, focus targets, and
//! toasts. Pure code motion from `mod.rs`.

use super::*;

pub(super) const MAX_VISIBLE_TOASTS: usize = 5;

pub(super) const TOAST_LIFETIME_MS: u64 = 5_000;

pub(super) const TOAST_ANIM_MS: u64 = 150;

pub(super) const CURSOR_BLINK_INTERVAL_MS: u64 = 530;

pub(super) const DEFAULT_UI_SCALE_PCT: u16 = 100;

pub(super) const MIN_UI_SCALE_PCT: u16 = 70;

pub(super) const MAX_UI_SCALE_PCT: u16 = 180;

pub(super) const UI_SCALE_STEP_PCT: u16 = 10;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum AppView {
    #[default]
    Workspace,
    Settings,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SettingsSection {
    #[default]
    Appearance,
    Editor,
    Behavior,
    Keymaps,
    Clankers,
    About,
}

impl SettingsSection {
    pub fn label(self) -> &'static str {
        match self {
            Self::Appearance => "Appearance",
            Self::Editor => "Editor",
            Self::Behavior => "Behavior",
            Self::Keymaps => "Keymaps",
            Self::Clankers => "Clankers",
            Self::About => "About",
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Self::Appearance => lucide::SUN,
            Self::Editor => lucide::FILE_CODE,
            Self::Behavior => lucide::SETTINGS,
            Self::Keymaps => lucide::KEY,
            Self::Clankers => lucide::SPARKLES,
            Self::About => lucide::INFO,
        }
    }

    pub const ALL: [Self; 6] = [
        Self::Appearance,
        Self::Editor,
        Self::Behavior,
        Self::Keymaps,
        Self::Clankers,
        Self::About,
    ];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusTarget {
    WorkspacePrimaryButton,
    TitleBar,
    ThemeToggle,
    FileList,
    Editor,
    PickerInput,
    PickerList,
    CommandPaletteInput,
    CommandPaletteList,
    AuthPrimaryAction,
    SidebarSearch,
    SearchInput,
    CommitEditor,
    ReviewCommentEditor,
    TextCompareLeft,
    TextCompareRight,
    SettingsOpenAiKey,
    SettingsAnthropicKey,
    SettingsSteeringPrompt,
}

impl FocusTarget {
    pub fn is_text_field(self) -> bool {
        matches!(
            self,
            Self::PickerInput
                | Self::CommandPaletteInput
                | Self::SidebarSearch
                | Self::SearchInput
                | Self::CommitEditor
                | Self::ReviewCommentEditor
                | Self::TextCompareLeft
                | Self::TextCompareRight
                | Self::SettingsOpenAiKey
                | Self::SettingsAnthropicKey
                | Self::SettingsSteeringPrompt
        )
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Toast {
    pub id: u64,
    pub kind: ToastKind,
    pub message: String,
    pub description: Option<String>,
    pub created_at_ms: u64,
    pub hovered: bool,
    /// When `Some`, the toast renders an externally-driven progress bar in
    /// place of the time-based one and is pinned (not auto-dismissed).
    pub progress: Option<f32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastKind {
    Info,
    Error,
}

/// App-chrome reactive state: view routing, focus, toasts, errors,
/// settings-page scroll metrics, theme preview, and the update lifecycle.
/// `#[derive(Store)]` turns every field into a `Signal` in the generated
/// `UiStateStore` held by `AppState`.
#[derive(Debug, Clone, Store)]
pub struct UiState {
    pub app_view: AppView,
    pub settings_section: SettingsSection,
    pub keymap_capture: Option<crate::input::ShortcutCommand>,
    pub keymaps_scroll_top_px: f32,
    pub keymaps_viewport_height_px: f32,
    pub keymaps_content_height_px: f32,
    pub focus: Option<FocusTarget>,
    pub last_error: Option<String>,
    pub toasts: Vec<Toast>,
    pub syntax_pack_installs: Vec<String>,
    pub update: UpdateState,
    pub sidebar_visible: bool,
    pub theme_preview_original: Option<String>,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            app_view: AppView::default(),
            settings_section: SettingsSection::default(),
            keymap_capture: None,
            keymaps_scroll_top_px: 0.0,
            keymaps_viewport_height_px: 0.0,
            keymaps_content_height_px: 0.0,
            focus: None,
            last_error: None,
            toasts: Vec::new(),
            syntax_pack_installs: Vec::new(),
            update: UpdateState::default(),
            sidebar_visible: true,
            theme_preview_original: None,
        }
    }
}

impl AppState {
    pub fn window_title(&self) -> String {
        let workspace_mode = if self
            .workspace
            .compare_progress
            .with(&self.store, |p| p.is_some())
        {
            "loading"
        } else {
            workspace_mode_name(self.workspace.mode.get(&self.store))
        };
        let title_prefix = crate::platform::startup::window_title_prefix();
        if self.workspace.source.get(&self.store) == WorkspaceSource::TextCompare {
            return format!("{title_prefix} - Text Compare [{workspace_mode}]");
        }
        let repo = self.compare.repo_path.with(&self.store, |p| {
            p.as_deref()
                .and_then(Path::file_name)
                .and_then(|value| value.to_str())
                .unwrap_or("native")
                .to_owned()
        });
        let selected_path = self.workspace.selected_file_path.get(&self.store);
        if let Some(path) = selected_path.as_deref() {
            format!("{title_prefix} - {repo} [{workspace_mode}] {path}")
        } else {
            format!("{title_prefix} - {repo} [{workspace_mode}]")
        }
    }

    pub fn update_time(&mut self, now_ms: u64) {
        self.clock_ms = now_ms;
        self.animation.tick(now_ms);
        let has_expired_toast = self.ui.toasts.with(&self.store, |toasts| {
            toasts.iter().any(|toast| {
                !toast.hovered
                    && toast.progress.is_none()
                    && now_ms.saturating_sub(toast.created_at_ms) >= TOAST_LIFETIME_MS
            })
        });
        if has_expired_toast {
            self.ui.toasts.update(&self.store, |toasts| {
                toasts.retain(|toast| {
                    toast.hovered
                        || toast.progress.is_some()
                        || now_ms.saturating_sub(toast.created_at_ms) < TOAST_LIFETIME_MS
                });
            });
        }
    }

    pub fn update_polling_enabled(&self) -> bool {
        self.settings.auto_update
            && crate::core::update::updates_configured()
            && !cfg!(debug_assertions)
            && !matches!(
                self.ui.update.get(&self.store),
                UpdateState::Downloading(_)
                    | UpdateState::ReadyToRestart(_)
                    | UpdateState::Restarting(_)
            )
    }

    pub fn cursor_blink_epoch(&self) -> Option<u64> {
        self.is_text_focused().then(|| {
            self.clock_ms
                .saturating_sub(self.text_edit.cursor_moved_at_ms.get(&self.store))
                / CURSOR_BLINK_INTERVAL_MS
        })
    }

    pub fn next_cursor_blink_at_ms(&self) -> Option<u64> {
        self.is_text_focused().then(|| {
            let moved_at = self.text_edit.cursor_moved_at_ms.get(&self.store);
            let elapsed = self.clock_ms.saturating_sub(moved_at);
            let next_epoch = elapsed / CURSOR_BLINK_INTERVAL_MS + 1;
            moved_at.saturating_add(next_epoch.saturating_mul(CURSOR_BLINK_INTERVAL_MS))
        })
    }

    pub fn next_toast_expiry_at_ms(&self) -> Option<u64> {
        self.ui.toasts.with(&self.store, |toasts| {
            toasts
                .iter()
                .filter(|toast| !toast.hovered && toast.progress.is_none())
                .map(|toast| toast.created_at_ms.saturating_add(TOAST_LIFETIME_MS))
                .min()
        })
    }

    pub(super) fn set_focus(&mut self, target: Option<FocusTarget>) {
        if target != self.ui.focus.get(&self.store) {
            // Reset cursor to end of the new field
            let len = target
                .and_then(|t| self.with_text_for_focus(t, |s| s.len()))
                .unwrap_or(0);
            self.reset_text_edit(len);
        }
        self.ui.focus.set(&self.store, target);
        self.editor
            .focused
            .set(&self.store, target == Some(FocusTarget::Editor));
    }

    /// Returns true if the current focus target is a text editing field.
    /// Backed by a memo; `focus` writes invalidate it automatically.
    pub fn is_text_focused(&self) -> bool {
        self.text_focused.get(&self.store)
    }

    pub(super) fn push_error(&mut self, message: &str) -> u64 {
        self.ui
            .last_error
            .set(&self.store, Some(message.to_owned()));
        self.push_toast(ToastKind::Error, message, None, None)
    }

    pub(super) fn push_info(&mut self, message: &str) -> u64 {
        self.push_toast(ToastKind::Info, message, None, None)
    }

    #[allow(dead_code)]
    pub(super) fn push_error_with_description(&mut self, message: &str, description: &str) -> u64 {
        self.ui
            .last_error
            .set(&self.store, Some(message.to_owned()));
        self.push_toast(
            ToastKind::Error,
            message,
            Some(description.to_owned()),
            None,
        )
    }

    #[allow(dead_code)]
    pub(super) fn push_info_with_description(&mut self, message: &str, description: &str) -> u64 {
        self.push_toast(ToastKind::Info, message, Some(description.to_owned()), None)
    }

    /// Create an info toast with an externally-driven progress bar (0.0-1.0).
    /// The toast is pinned until `finish_progress_toast` or `fail_progress_toast`
    /// is called — it does not auto-dismiss based on time.
    pub(super) fn push_progress_toast(&mut self, message: &str) -> u64 {
        self.push_toast(ToastKind::Info, message, None, Some(0.0))
    }

    /// Convert a pinned progress toast into a normal info toast and let it
    /// auto-dismiss. Also updates its message and description.
    pub(super) fn finish_progress_toast(
        &mut self,
        toast_id: u64,
        message: &str,
        description: Option<String>,
    ) {
        let now = self.clock_ms;
        self.ui.toasts.update(&self.store, |toasts| {
            if let Some(toast) = toasts.iter_mut().find(|t| t.id == toast_id) {
                toast.kind = ToastKind::Info;
                toast.message = message.to_owned();
                toast.description = description;
                toast.created_at_ms = now;
                toast.progress = None;
            }
        });
    }

    /// Convert a pinned progress toast into an error toast.
    pub(super) fn fail_progress_toast(
        &mut self,
        toast_id: u64,
        message: &str,
        description: Option<String>,
    ) {
        let now = self.clock_ms;
        self.ui
            .last_error
            .set(&self.store, Some(message.to_owned()));
        self.ui.toasts.update(&self.store, |toasts| {
            if let Some(toast) = toasts.iter_mut().find(|t| t.id == toast_id) {
                toast.kind = ToastKind::Error;
                toast.message = message.to_owned();
                toast.description = description;
                toast.created_at_ms = now;
                toast.progress = None;
            }
        });
    }

    pub(super) fn update_toast_progress(&mut self, toast_id: u64, fraction: f32) {
        let clamped = fraction.clamp(0.0, 1.0);
        self.ui.toasts.update(&self.store, |toasts| {
            if let Some(toast) = toasts.iter_mut().find(|t| t.id == toast_id) {
                toast.progress = Some(clamped);
            }
        });
    }

    pub(super) fn update_toast_message(&mut self, toast_id: u64, message: &str) {
        self.ui.toasts.update(&self.store, |toasts| {
            if let Some(toast) = toasts.iter_mut().find(|t| t.id == toast_id) {
                toast.message = message.to_owned();
            }
        });
    }

    pub(super) fn push_toast(
        &mut self,
        kind: ToastKind,
        message: &str,
        description: Option<String>,
        progress: Option<f32>,
    ) -> u64 {
        use crate::ui::animation::AnimationKey;
        let id = self.next_toast_id;
        self.next_toast_id = self.next_toast_id.saturating_add(1);
        self.animation.set_target(
            AnimationKey::ToastEntrance(id),
            1.0,
            TOAST_ANIM_MS,
            self.clock_ms,
        );
        let now = self.clock_ms;
        self.ui.toasts.update(&self.store, |toasts| {
            toasts.push(Toast {
                id,
                kind,
                message: message.to_owned(),
                description,
                created_at_ms: now,
                hovered: false,
                progress,
            });
            if toasts.len() > MAX_VISIBLE_TOASTS {
                toasts.remove(0);
            }
        });
        id
    }
}

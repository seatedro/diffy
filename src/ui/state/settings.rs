use crate::actions::SettingsAction;
use crate::effects::Effect;
use crate::events::SettingsEvent;

use super::*;

pub(super) fn reduce_action(state: &mut AppState, action: SettingsAction) -> Vec<Effect> {
    state.apply_settings_action(action)
}

pub(super) fn reduce_event(state: &mut AppState, event: SettingsEvent) -> Vec<Effect> {
    match event {
        SettingsEvent::SettingsSaved => Vec::new(),
        SettingsEvent::SettingsSaveFailed { message } => {
            state.push_error(&message);
            Vec::new()
        }
    }
}

impl AppState {
    pub(super) fn apply_settings_action(&mut self, action: SettingsAction) -> Vec<Effect> {
        use SettingsAction::*;
        match action {
            ToggleWrap => {
                let current = self.editor.wrap_enabled.get(&self.store);
                self.editor.wrap_enabled.set(&self.store, !current);
                self.persist_settings_effect()
            }
            SetWrapColumn(column) => {
                self.editor.wrap_column.set(&self.store, column);
                self.persist_settings_effect()
            }
            SetSidebarWidthPx(width) => {
                self.settings.sidebar_width_px = Some(self.clamp_sidebar_width_px(width));
                Vec::new()
            }
            IncreaseUiScale => self.adjust_ui_scale(UI_SCALE_STEP_PCT as i16),
            DecreaseUiScale => self.adjust_ui_scale(-(UI_SCALE_STEP_PCT as i16)),
            SetUiScalePct(pct) => {
                let clamped = self.clamp_ui_scale_pct(pct);
                if clamped == self.settings.ui_scale_pct {
                    return Vec::new();
                }
                self.settings.ui_scale_pct = clamped;
                self.persist_settings_effect()
            }
            ToggleThemeMode => {
                self.settings.theme_mode = match self.settings.theme_mode {
                    ThemeMode::Dark => ThemeMode::Light,
                    ThemeMode::Light => ThemeMode::Dark,
                };
                self.persist_settings_effect()
            }
            SetThemeMode(mode) => {
                if self.settings.theme_mode == mode {
                    return Vec::new();
                }
                self.settings.theme_mode = mode;
                self.persist_settings_effect()
            }
            SetThemeName(name) => {
                self.settings.theme_name = name;
                self.persist_settings_effect()
            }
            OpenUiFontPicker => {
                self.open_font_picker(crate::fonts::FontRole::Ui);
                Vec::new()
            }
            OpenMonoFontPicker => {
                self.open_font_picker(crate::fonts::FontRole::Mono);
                Vec::new()
            }
            SetUiFontFamily(family) => {
                let family =
                    crate::fonts::normalize_font_selection(crate::fonts::FontRole::Ui, &family);
                if self.settings.fonts.ui_family == family {
                    return Vec::new();
                }
                self.settings.fonts.ui_family = family;
                self.persist_settings_effect()
            }
            SetMonoFontFamily(family) => {
                let family =
                    crate::fonts::normalize_font_selection(crate::fonts::FontRole::Mono, &family);
                if self.settings.fonts.mono_family == family {
                    return Vec::new();
                }
                self.settings.fonts.mono_family = family;
                self.persist_settings_effect()
            }
            SetWheelScrollLines(lines) => {
                let clamped = lines.clamp(1, 10);
                if clamped == self.settings.wheel_scroll_lines {
                    return Vec::new();
                }
                self.settings.wheel_scroll_lines = clamped;
                self.persist_settings_effect()
            }
            ToggleContinuousScroll => {
                let enabled = !self.settings.continuous_scroll;
                self.settings.continuous_scroll = enabled;
                self.recompute_file_scroll_total_height_px();
                if enabled {
                    self.editor
                        .line_selection
                        .update(&self.store, |ls| ls.clear());
                    self.sync_global_scroll_from_editor();
                } else {
                    self.end_viewport_scrollbar_drag();
                }
                self.persist_settings_effect()
            }
            OpenThemePicker => {
                self.open_theme_picker();
                Vec::new()
            }
            OpenSettings => {
                self.clear_overlays();
                self.ui.app_view.set(&self.store, AppView::Settings);
                Vec::new()
            }
            OpenKeymaps => {
                self.clear_overlays();
                self.ui.app_view.set(&self.store, AppView::Settings);
                self.ui
                    .settings_section
                    .set(&self.store, SettingsSection::Keymaps);
                self.ui.keymaps_scroll_top_px.set(&self.store, 0.0);
                Vec::new()
            }
            CloseSettings => {
                self.ui.keymap_capture.set(&self.store, None);
                self.ui.app_view.set(&self.store, AppView::Workspace);
                Vec::new()
            }
            ToggleAutoUpdate => {
                self.settings.auto_update = !self.settings.auto_update;
                let mut effects = vec![SettingsEffect::SaveSettings(self.settings.clone()).into()];
                if self.update_polling_enabled() {
                    effects.push(UpdateEffect::CheckForUpdates { silent: true }.into());
                }
                effects
            }
            SetSettingsSection(section) => {
                self.ui.keymap_capture.set(&self.store, None);
                self.ui.settings_section.set(&self.store, section);
                self.ui.keymaps_scroll_top_px.set(&self.store, 0.0);
                Vec::new()
            }
            BeginKeymapRebind(command) => {
                self.ui.keymap_capture.set(&self.store, Some(command));
                Vec::new()
            }
            ApplyKeymapBinding { command, binding } => {
                crate::input::set_override(&mut self.settings.keymap_overrides, command, binding);
                self.ui.keymap_capture.set(&self.store, None);
                self.persist_settings_effect()
            }
            ResetKeymapBinding(command) => {
                crate::input::reset_override(&mut self.settings.keymap_overrides, command);
                self.ui.keymap_capture.set(&self.store, None);
                self.persist_settings_effect()
            }
            CancelKeymapRebind => {
                self.ui.keymap_capture.set(&self.store, None);
                Vec::new()
            }
            ScrollKeymapsPx(delta) => {
                let cur = self.ui.keymaps_scroll_top_px.get(&self.store);
                self.ui
                    .keymaps_scroll_top_px
                    .set(&self.store, cur + delta as f32);
                self.clamp_keymaps_scroll();
                Vec::new()
            }
            ScrollKeymapsToPx(target) => {
                self.ui
                    .keymaps_scroll_top_px
                    .set(&self.store, target as f32);
                self.clamp_keymaps_scroll();
                Vec::new()
            }
        }
    }
}

impl AppState {
    pub fn keymaps_max_scroll_px(&self) -> f32 {
        let content = self.ui.keymaps_content_height_px.get(&self.store);
        let viewport = self.ui.keymaps_viewport_height_px.get(&self.store);
        (content - viewport).max(0.0)
    }

    pub fn clamp_keymaps_scroll(&mut self) {
        let max = self.keymaps_max_scroll_px();
        let cur = self.ui.keymaps_scroll_top_px.get(&self.store);
        self.ui
            .keymaps_scroll_top_px
            .set(&self.store, cur.clamp(0.0, max));
    }
}

impl AppState {
    pub(super) fn persist_settings_effect(&mut self) -> Vec<Effect> {
        self.sync_settings_snapshot();
        vec![SettingsEffect::SaveSettings(self.settings.clone()).into()]
    }

    pub(super) fn sync_settings_snapshot(&mut self) {
        self.settings.ui_scale_pct = self.clamp_ui_scale_pct(self.settings.ui_scale_pct);
        self.settings.fonts = self.settings.fonts.normalized();
        self.settings.sidebar_width_px = self
            .settings
            .sidebar_width_px
            .map(|width| self.clamp_sidebar_width_px(width));
        self.settings.viewport.wrap_enabled = self.editor.wrap_enabled.get(&self.store);
        self.settings.viewport.wrap_column = self.editor.wrap_column.get(&self.store);
        self.settings.viewport.layout = self.compare.layout.get(&self.store);
        self.settings.last_compare = Some(PersistedCompare {
            repo_path: self.compare.repo_path.get(&self.store),
            left_ref: self.compare.left_ref.get(&self.store),
            right_ref: self.compare.right_ref.get(&self.store),
            mode: self.compare.mode.get(&self.store),
            layout: self.compare.layout.get(&self.store),
            renderer: self.compare.renderer.get(&self.store),
        });
    }

    pub fn ui_scale_factor(&self) -> f32 {
        self.clamp_ui_scale_pct(self.settings.ui_scale_pct) as f32 / DEFAULT_UI_SCALE_PCT as f32
    }

    pub(super) fn clamp_ui_scale_pct(&self, scale_pct: u16) -> u16 {
        scale_pct.clamp(MIN_UI_SCALE_PCT, MAX_UI_SCALE_PCT)
    }

    pub(super) fn adjust_ui_scale(&mut self, delta_pct: i16) -> Vec<Effect> {
        let current = i32::from(self.clamp_ui_scale_pct(self.settings.ui_scale_pct));
        let updated = (current + i32::from(delta_pct))
            .clamp(i32::from(MIN_UI_SCALE_PCT), i32::from(MAX_UI_SCALE_PCT))
            as u16;
        if updated == self.settings.ui_scale_pct {
            return Vec::new();
        }
        self.settings.ui_scale_pct = updated;
        self.persist_settings_effect()
    }
}

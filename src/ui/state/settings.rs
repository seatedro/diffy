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
                self.app_view.set(&self.store, AppView::Settings);
                Vec::new()
            }
            OpenKeymaps => {
                self.clear_overlays();
                self.app_view.set(&self.store, AppView::Settings);
                self.settings_section
                    .set(&self.store, SettingsSection::Keymaps);
                self.keymaps_scroll_top_px.set(&self.store, 0.0);
                Vec::new()
            }
            CloseSettings => {
                self.keymap_capture.set(&self.store, None);
                self.app_view.set(&self.store, AppView::Workspace);
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
                self.keymap_capture.set(&self.store, None);
                self.settings_section.set(&self.store, section);
                self.keymaps_scroll_top_px.set(&self.store, 0.0);
                Vec::new()
            }
            BeginKeymapRebind(command) => {
                self.keymap_capture.set(&self.store, Some(command));
                Vec::new()
            }
            ApplyKeymapBinding { command, binding } => {
                crate::input::set_override(&mut self.settings.keymap_overrides, command, binding);
                self.keymap_capture.set(&self.store, None);
                self.persist_settings_effect()
            }
            ResetKeymapBinding(command) => {
                crate::input::reset_override(&mut self.settings.keymap_overrides, command);
                self.keymap_capture.set(&self.store, None);
                self.persist_settings_effect()
            }
            CancelKeymapRebind => {
                self.keymap_capture.set(&self.store, None);
                Vec::new()
            }
            ScrollKeymapsPx(delta) => {
                let cur = self.keymaps_scroll_top_px.get(&self.store);
                self.keymaps_scroll_top_px
                    .set(&self.store, cur + delta as f32);
                self.clamp_keymaps_scroll();
                Vec::new()
            }
            ScrollKeymapsToPx(target) => {
                self.keymaps_scroll_top_px.set(&self.store, target as f32);
                self.clamp_keymaps_scroll();
                Vec::new()
            }
        }
    }
}

use winit::event::KeyEvent;
use winit::keyboard::{ModifiersState, NamedKey};

use crate::actions::{
    Action, AppAction, CompareAction, EditorAction, FileListAction, GitHubAction, OverlayAction,
    RepositoryAction, SettingsAction, TextEditAction,
};
use crate::ui::state::{
    AppState, AppView, FocusTarget, OverlaySurface, WorkspaceMode, WorkspaceSource,
};

use super::{InputContext, InputOutcome, InputOwner, InputSystem, KeyChord};

impl InputSystem {
    pub(super) fn route_text_input(&mut self, state: &AppState, text: String) -> InputOutcome {
        let ctx = super::resolve_input_context(state, self.ime_composing);
        match ctx.owner {
            InputOwner::TextField(_) if !text.is_empty() => {
                InputOutcome::action(TextEditAction::InsertText(text).into())
            }
            _ => InputOutcome::default(),
        }
    }

    pub(super) fn route_key_press(&mut self, state: &AppState, chord: KeyChord) -> InputOutcome {
        if chord.logical_char() != Some("g") {
            self.pending_g = false;
        }

        if let Some(action) = global_shortcut_action(&chord) {
            return InputOutcome::action(action);
        }

        let ctx = super::resolve_input_context(state, self.ime_composing);
        if matches!(ctx.owner, InputOwner::TextField(_))
            && let Some(action) = clipboard_shortcut_action(&chord)
        {
            return InputOutcome::action(action);
        }

        let actions = match ctx.owner {
            InputOwner::TextField(target) => {
                text_field_key_actions(&ctx, target, &chord).or_else(|| {
                    ctx.overlay
                        .and_then(|surface| overlay_key_actions(state, surface, &chord))
                })
            }
            InputOwner::Overlay(surface) => overlay_key_actions(state, surface, &chord),
            InputOwner::Editor => editor_key_actions(self, state, &chord),
            InputOwner::Workspace => workspace_key_actions(self, state, &chord),
        };

        InputOutcome::actions(actions.unwrap_or_default())
    }
}

fn global_shortcut_action(chord: &KeyChord) -> Option<Action> {
    if !chord.ctrl_or_super() {
        return None;
    }
    match chord.logical_char()?.to_ascii_lowercase().as_str() {
        "f" => Some(EditorAction::OpenSearch.into()),
        "p" => Some(OverlayAction::OpenCommandPalette.into()),
        "=" | "+" => Some(SettingsAction::IncreaseUiScale.into()),
        "-" | "_" => Some(SettingsAction::DecreaseUiScale.into()),
        "b" => Some(FileListAction::ToggleSidebar.into()),
        "," => Some(SettingsAction::OpenSettings.into()),
        _ => None,
    }
}

fn clipboard_shortcut_action(chord: &KeyChord) -> Option<Action> {
    if !chord.ctrl_or_super() {
        return None;
    }
    match chord.logical_char()?.to_ascii_lowercase().as_str() {
        "a" => Some(TextEditAction::SelectAll.into()),
        "c" => Some(TextEditAction::Copy.into()),
        "x" => Some(TextEditAction::Cut.into()),
        "v" => arboard::Clipboard::new()
            .ok()
            .and_then(|mut clipboard| clipboard.get_text().ok())
            .map(|text| TextEditAction::Paste(text).into()),
        _ => None,
    }
}

fn text_field_key_actions(
    ctx: &InputContext,
    target: FocusTarget,
    chord: &KeyChord,
) -> Option<Vec<Action>> {
    match chord.named() {
        Some(NamedKey::Enter) if target == FocusTarget::CommitEditor => {
            if chord.ctrl_or_super() {
                Some(vec![RepositoryAction::SubmitCommit.into()])
            } else {
                Some(vec![TextEditAction::InsertText("\n".to_owned()).into()])
            }
        }
        Some(NamedKey::Enter) if target == FocusTarget::SearchInput => {
            Some(vec![if chord.shift() {
                EditorAction::SearchPrevious.into()
            } else {
                EditorAction::SearchNext.into()
            }])
        }
        Some(NamedKey::ArrowUp) if target == FocusTarget::CommitEditor => {
            Some(vec![if chord.shift() {
                TextEditAction::SelectUp.into()
            } else {
                TextEditAction::CursorUp.into()
            }])
        }
        Some(NamedKey::ArrowDown) if target == FocusTarget::CommitEditor => {
            Some(vec![if chord.shift() {
                TextEditAction::SelectDown.into()
            } else {
                TextEditAction::CursorDown.into()
            }])
        }
        Some(NamedKey::ArrowLeft) => {
            let is_mac = cfg!(target_os = "macos");
            Some(vec![
                match (chord.ctrl_or_super(), chord.alt(), chord.shift()) {
                    (true, _, true) if is_mac => TextEditAction::SelectSoftHome.into(),
                    (true, _, false) if is_mac => TextEditAction::CursorSoftHome.into(),
                    (_, true, true) if is_mac => TextEditAction::SelectWordLeft.into(),
                    (_, true, false) if is_mac => TextEditAction::CursorWordLeft.into(),
                    (true, _, true) => TextEditAction::SelectWordLeft.into(),
                    (true, _, false) => TextEditAction::CursorWordLeft.into(),
                    (_, _, true) => TextEditAction::SelectLeft.into(),
                    (_, _, false) => TextEditAction::CursorLeft.into(),
                },
            ])
        }
        Some(NamedKey::ArrowRight) => {
            let is_mac = cfg!(target_os = "macos");
            Some(vec![
                match (chord.ctrl_or_super(), chord.alt(), chord.shift()) {
                    (true, _, true) if is_mac => TextEditAction::SelectSoftEnd.into(),
                    (true, _, false) if is_mac => TextEditAction::CursorSoftEnd.into(),
                    (_, true, true) if is_mac => TextEditAction::SelectWordRight.into(),
                    (_, true, false) if is_mac => TextEditAction::CursorWordRight.into(),
                    (true, _, true) => TextEditAction::SelectWordRight.into(),
                    (true, _, false) => TextEditAction::CursorWordRight.into(),
                    (_, _, true) => TextEditAction::SelectRight.into(),
                    (_, _, false) => TextEditAction::CursorRight.into(),
                },
            ])
        }
        Some(NamedKey::Home) => Some(vec![if chord.shift() {
            TextEditAction::SelectHome.into()
        } else {
            TextEditAction::CursorHome.into()
        }]),
        Some(NamedKey::End) => Some(vec![if chord.shift() {
            TextEditAction::SelectEnd.into()
        } else {
            TextEditAction::CursorEnd.into()
        }]),
        Some(NamedKey::Backspace) => {
            let is_mac = cfg!(target_os = "macos");
            Some(vec![if chord.ctrl_or_super() && is_mac {
                TextEditAction::BackspaceLine.into()
            } else if chord.alt() && is_mac || chord.ctrl_or_super() && !is_mac {
                TextEditAction::BackspaceWord.into()
            } else {
                TextEditAction::Backspace.into()
            }])
        }
        Some(NamedKey::Delete) => {
            let is_mac = cfg!(target_os = "macos");
            Some(vec![
                if chord.alt() && is_mac || chord.ctrl_or_super() && !is_mac {
                    TextEditAction::DeleteForwardWord.into()
                } else {
                    TextEditAction::DeleteForward.into()
                },
            ])
        }
        Some(NamedKey::Escape) if ctx.overlay.is_some() => {
            Some(vec![OverlayAction::CloseOverlay.into()])
        }
        _ => None,
    }
}

fn overlay_key_actions(
    state: &AppState,
    surface: OverlaySurface,
    chord: &KeyChord,
) -> Option<Vec<Action>> {
    match chord.named() {
        Some(NamedKey::Escape) => Some(vec![OverlayAction::CloseOverlay.into()]),
        Some(NamedKey::Tab) => {
            if surface == OverlaySurface::RepoPicker {
                Some(vec![OverlayAction::TabCompletePickerDir.into()])
            } else {
                Some(vec![AppAction::SetFocus(cycle_focus_target(state)).into()])
            }
        }
        Some(NamedKey::Enter) => activate_current_focus_actions(state),
        Some(NamedKey::ArrowDown) => Some(vec![OverlayAction::MoveOverlaySelection(1).into()]),
        Some(NamedKey::ArrowUp) => Some(vec![OverlayAction::MoveOverlaySelection(-1).into()]),
        _ => None,
    }
}

fn editor_key_actions(
    input: &mut InputSystem,
    state: &AppState,
    chord: &KeyChord,
) -> Option<Vec<Action>> {
    workspace_key_actions_inner(Some(input), state, chord)
}

fn workspace_key_actions(
    input: &mut InputSystem,
    state: &AppState,
    chord: &KeyChord,
) -> Option<Vec<Action>> {
    workspace_key_actions_inner(Some(input), state, chord)
}

fn workspace_key_actions_inner(
    mut input: Option<&mut InputSystem>,
    state: &AppState,
    chord: &KeyChord,
) -> Option<Vec<Action>> {
    match chord.named() {
        Some(NamedKey::Escape) => {
            if state.overlays_top().is_some() {
                Some(vec![OverlayAction::CloseOverlay.into()])
            } else if state.app_view.get(&state.store) == AppView::Settings {
                Some(vec![SettingsAction::CloseSettings.into()])
            } else if state.editor.search.open.get(&state.store) {
                Some(vec![EditorAction::CloseSearch.into()])
            } else if state.focus.get(&state.store) == Some(FocusTarget::SidebarSearch) {
                Some(vec![
                    FileListAction::ClearSidebarFilter.into(),
                    AppAction::SetFocus(None).into(),
                ])
            } else {
                None
            }
        }
        Some(NamedKey::Tab) => Some(vec![AppAction::SetFocus(cycle_focus_target(state)).into()]),
        Some(NamedKey::Enter) => {
            if state.focus.get(&state.store) == Some(FocusTarget::SearchInput) {
                Some(vec![if chord.shift() {
                    EditorAction::SearchPrevious.into()
                } else {
                    EditorAction::SearchNext.into()
                }])
            } else {
                activate_current_focus_actions(state)
            }
        }
        Some(NamedKey::ArrowDown) => {
            if state.focus.get(&state.store) == Some(FocusTarget::Editor) {
                Some(vec![EditorAction::ScrollViewportLines(1).into()])
            } else if state.is_workspace_ready() {
                Some(vec![FileListAction::SelectNextFile.into()])
            } else {
                None
            }
        }
        Some(NamedKey::ArrowUp) => {
            if state.focus.get(&state.store) == Some(FocusTarget::Editor) {
                Some(vec![EditorAction::ScrollViewportLines(-1).into()])
            } else if state.is_workspace_ready() {
                Some(vec![FileListAction::SelectPreviousFile.into()])
            } else {
                None
            }
        }
        Some(NamedKey::PageDown) if state.is_workspace_ready() => {
            if state.focus.get(&state.store) == Some(FocusTarget::Editor) {
                Some(vec![EditorAction::ScrollViewportPages(1).into()])
            } else {
                Some(vec![FileListAction::ScrollFileList(10).into()])
            }
        }
        Some(NamedKey::PageUp) if state.is_workspace_ready() => {
            if state.focus.get(&state.store) == Some(FocusTarget::Editor) {
                Some(vec![EditorAction::ScrollViewportPages(-1).into()])
            } else {
                Some(vec![FileListAction::ScrollFileList(-10).into()])
            }
        }
        Some(NamedKey::Home) if state.is_workspace_ready() => {
            let action = if state.settings.continuous_scroll {
                EditorAction::ScrollViewportToGlobal(0)
            } else {
                EditorAction::ScrollViewportTo(0)
            };
            Some(vec![action.into()])
        }
        Some(NamedKey::End) if state.is_workspace_ready() => {
            let action = if state.settings.continuous_scroll {
                EditorAction::ScrollViewportToGlobal(state.global_max_scroll_top_px())
            } else {
                EditorAction::ScrollViewportTo(state.editor_max_scroll_top_px())
            };
            Some(vec![action.into()])
        }
        _ => {
            let ch = chord.logical_char()?;
            if ch == "?" {
                return Some(vec![OverlayAction::ShowKeyboardShortcuts.into()]);
            }
            if state.overlays_top().is_some()
                || state.workspace_mode.get(&state.store) != WorkspaceMode::Ready
            {
                return None;
            }
            match ch {
                "/" => Some(vec![
                    AppAction::SetFocus(Some(FocusTarget::SidebarSearch)).into(),
                ]),
                "]" => Some(vec![EditorAction::GoToNextHunk.into()]),
                "[" => Some(vec![EditorAction::GoToPreviousHunk.into()]),
                "n" => Some(vec![EditorAction::GoToNextFile.into()]),
                "N" => Some(vec![EditorAction::GoToPreviousFile.into()]),
                "j" => Some(vec![EditorAction::ScrollViewportLines(1).into()]),
                "k" => Some(vec![EditorAction::ScrollViewportLines(-1).into()]),
                "d" => Some(vec![EditorAction::ScrollViewportHalfPage(1).into()]),
                "u" => Some(vec![EditorAction::ScrollViewportHalfPage(-1).into()]),
                "G" => {
                    let action = if state.settings.continuous_scroll {
                        EditorAction::ScrollViewportToGlobal(state.global_max_scroll_top_px())
                    } else {
                        EditorAction::ScrollViewportTo(state.editor_max_scroll_top_px())
                    };
                    Some(vec![action.into()])
                }
                "g" => {
                    let input = input.as_mut()?;
                    if input.pending_g {
                        input.pending_g = false;
                        let action = if state.settings.continuous_scroll {
                            EditorAction::ScrollViewportToGlobal(0)
                        } else {
                            EditorAction::ScrollViewportTo(0)
                        };
                        Some(vec![action.into()])
                    } else {
                        input.pending_g = true;
                        Some(Vec::new())
                    }
                }
                "1" => Some(vec![
                    CompareAction::SetLayoutMode(crate::core::compare::LayoutMode::Unified).into(),
                ]),
                "2" => Some(vec![
                    CompareAction::SetLayoutMode(crate::core::compare::LayoutMode::Split).into(),
                ]),
                "w" => Some(vec![SettingsAction::ToggleWrap.into()]),
                "s" if state.workspace.source.get(&state.store) == WorkspaceSource::Status => {
                    if state
                        .editor
                        .line_selection
                        .with(&state.store, |ls| ls.is_empty())
                    {
                        Some(vec![RepositoryAction::StageHunk.into()])
                    } else {
                        Some(vec![RepositoryAction::StageSelectedLines.into()])
                    }
                }
                "S" if state.workspace.source.get(&state.store) == WorkspaceSource::Status => {
                    if state
                        .editor
                        .line_selection
                        .with(&state.store, |ls| ls.is_empty())
                    {
                        Some(vec![RepositoryAction::UnstageHunk.into()])
                    } else {
                        Some(vec![RepositoryAction::UnstageSelectedLines.into()])
                    }
                }
                "x" if state.workspace.source.get(&state.store) == WorkspaceSource::Status => {
                    if state
                        .editor
                        .line_selection
                        .with(&state.store, |ls| ls.is_empty())
                    {
                        Some(vec![RepositoryAction::DiscardHunk.into()])
                    } else {
                        Some(vec![RepositoryAction::DiscardSelectedLines.into()])
                    }
                }
                " " => Some(vec![if chord.shift() {
                    EditorAction::ScrollViewportPages(-1).into()
                } else {
                    EditorAction::ScrollViewportPages(1).into()
                }]),
                _ => None,
            }
        }
    }
}

fn cycle_focus_target(state: &AppState) -> Option<FocusTarget> {
    match state.overlays_top() {
        Some(OverlaySurface::RepoPicker | OverlaySurface::RefPicker) => {
            match state.focus.get(&state.store) {
                Some(FocusTarget::PickerInput) => Some(FocusTarget::PickerList),
                _ => Some(FocusTarget::PickerInput),
            }
        }
        Some(OverlaySurface::CommandPalette) => match state.focus.get(&state.store) {
            Some(FocusTarget::CommandPaletteInput) => Some(FocusTarget::CommandPaletteList),
            _ => Some(FocusTarget::CommandPaletteInput),
        },
        Some(OverlaySurface::ThemePicker) => match state.focus.get(&state.store) {
            Some(FocusTarget::PickerInput) => Some(FocusTarget::PickerList),
            _ => Some(FocusTarget::PickerInput),
        },
        Some(OverlaySurface::GitHubAuthModal) => Some(FocusTarget::AuthPrimaryAction),
        Some(
            OverlaySurface::KeyboardShortcuts
            | OverlaySurface::CompareMenu
            | OverlaySurface::AccountMenu
            | OverlaySurface::PublishMenu,
        ) => None,
        None => match state.focus.get(&state.store) {
            Some(FocusTarget::FileList) => Some(FocusTarget::Editor),
            Some(FocusTarget::Editor) => Some(FocusTarget::FileList),
            Some(FocusTarget::WorkspacePrimaryButton) => Some(FocusTarget::TitleBar),
            _ => Some(if state.is_workspace_ready() {
                FocusTarget::FileList
            } else {
                FocusTarget::WorkspacePrimaryButton
            }),
        },
    }
}

fn activate_current_focus_actions(state: &AppState) -> Option<Vec<Action>> {
    match state.overlays_top() {
        Some(
            OverlaySurface::RepoPicker
            | OverlaySurface::RefPicker
            | OverlaySurface::CommandPalette
            | OverlaySurface::ThemePicker,
        ) => Some(vec![OverlayAction::ConfirmOverlaySelection.into()]),
        Some(OverlaySurface::GitHubAuthModal) => {
            let has_flow = state
                .github
                .auth
                .device_flow
                .with(&state.store, |opt| opt.is_some());
            Some(vec![if has_flow {
                GitHubAction::OpenDeviceFlowBrowser.into()
            } else {
                GitHubAction::StartGitHubDeviceFlow.into()
            }])
        }
        Some(
            OverlaySurface::KeyboardShortcuts
            | OverlaySurface::CompareMenu
            | OverlaySurface::AccountMenu
            | OverlaySurface::PublishMenu,
        ) => Some(Vec::new()),
        None => match state.focus.get(&state.store) {
            Some(FocusTarget::WorkspacePrimaryButton) => {
                Some(vec![OverlayAction::OpenRepoPicker.into()])
            }
            Some(FocusTarget::ThemeToggle) => Some(vec![SettingsAction::ToggleThemeMode.into()]),
            _ => None,
        },
    }
}

pub(super) fn key_text_from_key_event(
    event: &KeyEvent,
    modifiers: ModifiersState,
    ime_composing: bool,
) -> Option<String> {
    if ime_composing || modifiers.control_key() || modifiers.super_key() {
        return None;
    }
    let text = event.text.as_ref()?;
    if text.is_empty() || text.chars().all(char::is_control) {
        return None;
    }
    Some(text.to_string())
}

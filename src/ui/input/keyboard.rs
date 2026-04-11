use winit::event::KeyEvent;
use winit::keyboard::{ModifiersState, NamedKey};

use crate::actions::Action;
use crate::ui::state::{AppState, CompareField, FocusTarget, OverlaySurface, WorkspaceMode};

use super::{InputContext, InputOutcome, InputOwner, InputSystem, KeyChord};

impl InputSystem {
    pub(super) fn route_text_input(&mut self, state: &AppState, text: String) -> InputOutcome {
        let ctx = super::resolve_input_context(state, self.ime_composing);
        match ctx.owner {
            InputOwner::TextField(_) if !text.is_empty() => {
                InputOutcome::action(Action::InsertText(text))
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
        "f" => Some(Action::OpenSearch),
        "p" => Some(Action::OpenCommandPalette),
        "=" | "+" => Some(Action::IncreaseUiScale),
        "-" | "_" => Some(Action::DecreaseUiScale),
        "b" => Some(Action::ToggleSidebar),
        _ => None,
    }
}

fn clipboard_shortcut_action(chord: &KeyChord) -> Option<Action> {
    if !chord.ctrl_or_super() {
        return None;
    }
    match chord.logical_char()?.to_ascii_lowercase().as_str() {
        "a" => Some(Action::SelectAll),
        "c" => Some(Action::Copy),
        "x" => Some(Action::Cut),
        "v" => arboard::Clipboard::new()
            .ok()
            .and_then(|mut clipboard| clipboard.get_text().ok())
            .map(Action::Paste),
        _ => None,
    }
}

fn text_field_key_actions(
    ctx: &InputContext,
    target: FocusTarget,
    chord: &KeyChord,
) -> Option<Vec<Action>> {
    match chord.named() {
        Some(NamedKey::Enter) if target == FocusTarget::SearchInput => {
            Some(vec![if chord.shift() {
                Action::SearchPrevious
            } else {
                Action::SearchNext
            }])
        }
        Some(NamedKey::ArrowLeft) => Some(vec![match (chord.ctrl_or_super(), chord.shift()) {
            (true, true) => Action::SelectWordLeft,
            (true, false) => Action::CursorWordLeft,
            (false, true) => Action::SelectLeft,
            (false, false) => Action::CursorLeft,
        }]),
        Some(NamedKey::ArrowRight) => Some(vec![match (chord.ctrl_or_super(), chord.shift()) {
            (true, true) => Action::SelectWordRight,
            (true, false) => Action::CursorWordRight,
            (false, true) => Action::SelectRight,
            (false, false) => Action::CursorRight,
        }]),
        Some(NamedKey::Home) => Some(vec![if chord.shift() {
            Action::SelectHome
        } else {
            Action::CursorHome
        }]),
        Some(NamedKey::End) => Some(vec![if chord.shift() {
            Action::SelectEnd
        } else {
            Action::CursorEnd
        }]),
        Some(NamedKey::Backspace) => Some(vec![Action::Backspace]),
        Some(NamedKey::Delete) => Some(vec![Action::DeleteForward]),
        Some(NamedKey::Escape) if ctx.overlay.is_some() => Some(vec![Action::CloseOverlay]),
        _ => None,
    }
}

fn overlay_key_actions(
    state: &AppState,
    surface: OverlaySurface,
    chord: &KeyChord,
) -> Option<Vec<Action>> {
    match chord.named() {
        Some(NamedKey::Escape) => Some(vec![Action::CloseOverlay]),
        Some(NamedKey::Tab) => {
            if surface == OverlaySurface::RepoPicker {
                Some(vec![Action::TabCompletePickerDir])
            } else {
                Some(vec![Action::SetFocus(cycle_focus_target(state))])
            }
        }
        Some(NamedKey::Enter) => activate_current_focus_actions(state),
        Some(NamedKey::ArrowDown) => Some(vec![Action::MoveOverlaySelection(1)]),
        Some(NamedKey::ArrowUp) => Some(vec![Action::MoveOverlaySelection(-1)]),
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
            if state.overlays.top().is_some() {
                Some(vec![Action::CloseOverlay])
            } else if state.editor.search.open {
                Some(vec![Action::CloseSearch])
            } else if state.focus.current == Some(FocusTarget::SidebarSearch) {
                Some(vec![Action::ClearSidebarFilter, Action::SetFocus(None)])
            } else {
                None
            }
        }
        Some(NamedKey::Tab) => Some(vec![Action::SetFocus(cycle_focus_target(state))]),
        Some(NamedKey::Enter) => {
            if state.focus.current == Some(FocusTarget::SearchInput) {
                Some(vec![if chord.shift() {
                    Action::SearchPrevious
                } else {
                    Action::SearchNext
                }])
            } else {
                activate_current_focus_actions(state)
            }
        }
        Some(NamedKey::ArrowDown) => {
            if state.focus.current == Some(FocusTarget::Editor) {
                Some(vec![Action::ScrollViewportLines(1)])
            } else if state.workspace_mode == WorkspaceMode::Ready {
                Some(vec![Action::SelectNextFile])
            } else {
                None
            }
        }
        Some(NamedKey::ArrowUp) => {
            if state.focus.current == Some(FocusTarget::Editor) {
                Some(vec![Action::ScrollViewportLines(-1)])
            } else if state.workspace_mode == WorkspaceMode::Ready {
                Some(vec![Action::SelectPreviousFile])
            } else {
                None
            }
        }
        Some(NamedKey::PageDown) if state.workspace_mode == WorkspaceMode::Ready => {
            if state.focus.current == Some(FocusTarget::Editor) {
                Some(vec![Action::ScrollViewportPages(1)])
            } else {
                Some(vec![Action::ScrollFileList(10)])
            }
        }
        Some(NamedKey::PageUp) if state.workspace_mode == WorkspaceMode::Ready => {
            if state.focus.current == Some(FocusTarget::Editor) {
                Some(vec![Action::ScrollViewportPages(-1)])
            } else {
                Some(vec![Action::ScrollFileList(-10)])
            }
        }
        Some(NamedKey::Home) if state.workspace_mode == WorkspaceMode::Ready => {
            Some(vec![Action::ScrollViewportTo(0)])
        }
        Some(NamedKey::End) if state.workspace_mode == WorkspaceMode::Ready => {
            Some(vec![Action::ScrollViewportTo(
                state.editor.max_scroll_top_px(),
            )])
        }
        _ => {
            let ch = chord.logical_char()?;
            if ch == "?" {
                return Some(vec![Action::ShowKeyboardShortcuts]);
            }
            if state.overlays.top().is_some() || state.workspace_mode != WorkspaceMode::Ready {
                return None;
            }
            match ch {
                "/" => Some(vec![Action::SetFocus(Some(FocusTarget::SidebarSearch))]),
                "]" => Some(vec![Action::GoToNextHunk]),
                "[" => Some(vec![Action::GoToPreviousHunk]),
                "n" => Some(vec![Action::GoToNextFile]),
                "N" => Some(vec![Action::GoToPreviousFile]),
                "j" => Some(vec![Action::ScrollViewportLines(1)]),
                "k" => Some(vec![Action::ScrollViewportLines(-1)]),
                "d" => Some(vec![Action::ScrollViewportHalfPage(1)]),
                "u" => Some(vec![Action::ScrollViewportHalfPage(-1)]),
                "G" => Some(vec![Action::ScrollViewportTo(
                    state.editor.max_scroll_top_px(),
                )]),
                "g" => {
                    let input = input.as_mut()?;
                    if input.pending_g {
                        input.pending_g = false;
                        Some(vec![Action::ScrollViewportTo(0)])
                    } else {
                        input.pending_g = true;
                        Some(Vec::new())
                    }
                }
                "1" => Some(vec![Action::SetLayoutMode(
                    crate::core::compare::LayoutMode::Unified,
                )]),
                "2" => Some(vec![Action::SetLayoutMode(
                    crate::core::compare::LayoutMode::Split,
                )]),
                "w" => Some(vec![Action::ToggleWrap]),
                " " => Some(vec![if chord.shift() {
                    Action::ScrollViewportPages(-1)
                } else {
                    Action::ScrollViewportPages(1)
                }]),
                _ => None,
            }
        }
    }
}

fn cycle_focus_target(state: &AppState) -> Option<FocusTarget> {
    match state.overlays.top() {
        Some(OverlaySurface::CompareSheet) => match state.focus.current {
            Some(FocusTarget::CompareRepoButton) => Some(FocusTarget::CompareLeftRef),
            Some(FocusTarget::CompareLeftRef) => Some(FocusTarget::CompareRightRef),
            Some(FocusTarget::CompareRightRef) => Some(FocusTarget::CompareStartButton),
            _ => Some(FocusTarget::CompareRepoButton),
        },
        Some(OverlaySurface::RepoPicker | OverlaySurface::RefPicker(_)) => {
            match state.focus.current {
                Some(FocusTarget::PickerInput) => Some(FocusTarget::PickerList),
                _ => Some(FocusTarget::PickerInput),
            }
        }
        Some(OverlaySurface::CommandPalette) => match state.focus.current {
            Some(FocusTarget::CommandPaletteInput) => Some(FocusTarget::CommandPaletteList),
            _ => Some(FocusTarget::CommandPaletteInput),
        },
        Some(OverlaySurface::PullRequestModal) => match state.focus.current {
            Some(FocusTarget::PullRequestInput) => Some(FocusTarget::PullRequestConfirm),
            _ => Some(FocusTarget::PullRequestInput),
        },
        Some(OverlaySurface::ThemePicker) => match state.focus.current {
            Some(FocusTarget::PickerInput) => Some(FocusTarget::PickerList),
            _ => Some(FocusTarget::PickerInput),
        },
        Some(OverlaySurface::GitHubAuthModal) => Some(FocusTarget::AuthPrimaryAction),
        Some(OverlaySurface::KeyboardShortcuts) => None,
        None => match state.focus.current {
            Some(FocusTarget::FileList) => Some(FocusTarget::Editor),
            Some(FocusTarget::Editor) => Some(FocusTarget::FileList),
            Some(FocusTarget::WorkspacePrimaryButton) => Some(FocusTarget::TitleBar),
            _ => Some(if state.workspace_mode == WorkspaceMode::Ready {
                FocusTarget::FileList
            } else {
                FocusTarget::WorkspacePrimaryButton
            }),
        },
    }
}

fn activate_current_focus_actions(state: &AppState) -> Option<Vec<Action>> {
    match state.overlays.top() {
        Some(OverlaySurface::CompareSheet) => Some(match state.focus.current {
            Some(FocusTarget::CompareRepoButton) => vec![Action::OpenRepoPicker],
            Some(FocusTarget::CompareLeftRef) => vec![Action::OpenRefPicker(CompareField::Left)],
            Some(FocusTarget::CompareRightRef) => vec![Action::OpenRefPicker(CompareField::Right)],
            _ => vec![Action::StartCompare],
        }),
        Some(
            OverlaySurface::RepoPicker
            | OverlaySurface::RefPicker(_)
            | OverlaySurface::CommandPalette
            | OverlaySurface::ThemePicker,
        ) => Some(vec![Action::ConfirmOverlaySelection]),
        Some(OverlaySurface::PullRequestModal) => Some(vec![Action::SubmitPullRequest]),
        Some(OverlaySurface::GitHubAuthModal) => {
            Some(vec![if state.github.auth.device_flow.is_some() {
                Action::OpenDeviceFlowBrowser
            } else {
                Action::StartGitHubDeviceFlow
            }])
        }
        Some(OverlaySurface::KeyboardShortcuts) => Some(Vec::new()),
        None => match state.focus.current {
            Some(FocusTarget::WorkspacePrimaryButton) => Some(vec![Action::OpenCompareSheet]),
            Some(FocusTarget::ThemeToggle) => Some(vec![Action::ToggleThemeMode]),
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

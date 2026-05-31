use winit::event::KeyEvent;
use winit::keyboard::{ModifiersState, NamedKey};

use crate::actions::{
    Action, AppAction, CompareAction, EditorAction, FileListAction, GitHubAction, OverlayAction,
    RepositoryAction, SettingsAction, TextEditAction, UpdateAction, WorkspaceAction,
};
use crate::core::vcs::model::RefKind;
use crate::ui::editor::element::EditorElement;
use crate::ui::shell::UiFrame;
use crate::ui::state::{
    AppState, AppView, FocusTarget, OverlaySurface, SettingsSection, SidebarTab, WorkspaceMode,
    WorkspaceSource,
};

use super::{
    InputContext, InputOutcome, InputOwner, InputSystem, KeyChord, ShortcutCommand, binding_matches,
};

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

    pub(super) fn route_key_press(
        &mut self,
        state: &AppState,
        ui_frame: &UiFrame,
        editor: &EditorElement,
        chord: KeyChord,
    ) -> InputOutcome {
        if let Some(actions) = keymap_capture_actions(state, &chord) {
            return InputOutcome::actions(actions);
        }

        if chord.logical_char() != Some("g") {
            self.pending_g = false;
        }

        if state.context_menu.visible && chord.named() == Some(NamedKey::Escape) {
            return InputOutcome::action(AppAction::CloseContextMenu.into());
        }

        if let Some(action) = global_shortcut_action(state, &chord) {
            return InputOutcome::action(action);
        }

        let ctx = super::resolve_input_context(state, self.ime_composing);
        if matches!(ctx.owner, InputOwner::TextField(_))
            && let Some(action) = clipboard_shortcut_action(&chord)
        {
            return InputOutcome::action(action);
        }
        if let Some(actions) =
            viewport_clipboard_shortcut_actions(state, ui_frame, editor, &ctx, &chord)
        {
            return InputOutcome::actions(actions);
        }

        let actions = match ctx.owner {
            InputOwner::TextField(target) => text_field_key_actions(state, &ctx, target, &chord)
                .or_else(|| {
                    ctx.overlay
                        .and_then(|surface| overlay_key_actions(state, surface, &chord, false))
                }),
            InputOwner::Overlay(surface) => overlay_key_actions(state, surface, &chord, true),
            InputOwner::Editor => editor_key_actions(self, state, &chord),
            InputOwner::Workspace => workspace_key_actions(self, state, &chord),
        };

        InputOutcome::actions(actions.unwrap_or_default())
    }
}

fn viewport_clipboard_shortcut_actions(
    state: &AppState,
    ui_frame: &UiFrame,
    editor: &EditorElement,
    ctx: &InputContext,
    chord: &KeyChord,
) -> Option<Vec<Action>> {
    if !chord.ctrl_or_super() || chord.logical_char()?.to_ascii_lowercase() != "c" {
        return None;
    }
    if !matches!(ctx.owner, InputOwner::Editor | InputOwner::Workspace) {
        return None;
    }
    // A review-card text selection takes the clipboard (it is mutually exclusive
    // with the viewport selection, so at most one is ever non-empty).
    if let Some(text) = state
        .github
        .pull_request
        .card_text_selection
        .with(&state.store, |sel| {
            sel.as_ref().and_then(|sel| sel.selected_text())
        })
    {
        let mut actions = vec![AppAction::CopyText(text).into()];
        if state.context_menu.visible {
            actions.push(AppAction::CloseContextMenu.into());
        }
        return Some(actions);
    }
    let document = ui_frame.viewport_document.as_ref()?;
    let selection = state.editor.text_selection.get(&state.store)?;
    if selection.generation != document.generation {
        return None;
    }
    let text = editor.viewport_selection_text(document.doc.as_ref(), &selection)?;
    let mut actions = vec![AppAction::CopyText(text).into()];
    if state.context_menu.visible {
        actions.push(AppAction::CloseContextMenu.into());
    }
    Some(actions)
}

fn global_shortcut_action(state: &AppState, chord: &KeyChord) -> Option<Action> {
    let binding = chord.binding_string()?;
    let overrides = &state.settings.keymap_overrides;
    if matches_binding(overrides, ShortcutCommand::OpenSearch, &binding) {
        Some(EditorAction::OpenSearch.into())
    } else if matches_binding(overrides, ShortcutCommand::OpenCommandPalette, &binding) {
        Some(OverlayAction::OpenCommandPalette.into())
    } else if matches_binding(overrides, ShortcutCommand::IncreaseUiScale, &binding) {
        Some(SettingsAction::IncreaseUiScale.into())
    } else if matches_binding(overrides, ShortcutCommand::DecreaseUiScale, &binding) {
        Some(SettingsAction::DecreaseUiScale.into())
    } else if matches_binding(overrides, ShortcutCommand::ToggleSidebar, &binding) {
        Some(FileListAction::ToggleSidebar.into())
    } else if matches_binding(overrides, ShortcutCommand::OpenSettings, &binding) {
        Some(SettingsAction::OpenSettings.into())
    } else if matches_binding(overrides, ShortcutCommand::ShowKeymaps, &binding) {
        Some(SettingsAction::OpenKeymaps.into())
    } else {
        None
    }
}

fn keymap_capture_actions(state: &AppState, chord: &KeyChord) -> Option<Vec<Action>> {
    let command = state.keymap_capture.get(&state.store)?;
    if chord.named() == Some(NamedKey::Escape) {
        return Some(vec![SettingsAction::CancelKeymapRebind.into()]);
    }
    let binding = chord.binding_string()?;
    Some(vec![
        SettingsAction::ApplyKeymapBinding { command, binding }.into(),
    ])
}

fn matches_binding(
    overrides: &[crate::input::KeymapOverride],
    command: ShortcutCommand,
    binding: &str,
) -> bool {
    binding_matches(overrides, command, binding)
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
    state: &AppState,
    ctx: &InputContext,
    target: FocusTarget,
    chord: &KeyChord,
) -> Option<Vec<Action>> {
    if target == FocusTarget::CommitEditor
        && chord.binding_string().is_some_and(|binding| {
            matches_binding(
                &state.settings.keymap_overrides,
                ShortcutCommand::SubmitCommit,
                &binding,
            )
        })
    {
        return Some(vec![RepositoryAction::SubmitCommit.into()]);
    }

    match chord.named() {
        Some(NamedKey::Enter) if target == FocusTarget::CommitEditor => {
            if chord.ctrl_or_super() {
                Some(vec![RepositoryAction::SubmitCommit.into()])
            } else {
                Some(vec![TextEditAction::InsertText("\n".to_owned()).into()])
            }
        }
        Some(NamedKey::Enter) if target == FocusTarget::ReviewCommentEditor => {
            if chord.ctrl_or_super() {
                Some(vec![GitHubAction::SubmitReviewComment.into()])
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
        Some(NamedKey::Escape) if target == FocusTarget::SearchInput => {
            Some(vec![EditorAction::CloseSearch.into()])
        }
        Some(NamedKey::Escape) if target == FocusTarget::SidebarSearch => Some(vec![
            FileListAction::ClearSidebarFilter.into(),
            AppAction::SetFocus(None).into(),
        ]),
        Some(NamedKey::Escape)
            if matches!(
                target,
                FocusTarget::CommitEditor
                    | FocusTarget::SettingsOpenAiKey
                    | FocusTarget::SettingsAnthropicKey
                    | FocusTarget::SettingsSteeringPrompt
            ) =>
        {
            Some(vec![AppAction::SetFocus(None).into()])
        }
        Some(NamedKey::Escape) if target == FocusTarget::ReviewCommentEditor => {
            Some(vec![GitHubAction::CancelReviewComment.into()])
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
    allow_character_navigation: bool,
) -> Option<Vec<Action>> {
    if let Some(actions) = match chord.named() {
        Some(NamedKey::Escape) => Some(vec![OverlayAction::CloseOverlay.into()]),
        Some(NamedKey::Enter) if surface == OverlaySurface::PublishMenu => {
            publish_menu_action_at(state, 0)
        }
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
    } {
        return Some(actions);
    }

    if !allow_character_navigation {
        return None;
    }

    if surface == OverlaySurface::CompareMenu
        && let Some(index) = digit_shortcut_index(chord.logical_char()?)
    {
        return compare_menu_action_at(state, index);
    }

    if surface == OverlaySurface::PublishMenu
        && let Some(index) = digit_shortcut_index(chord.logical_char()?)
    {
        return publish_menu_action_at(state, index);
    }

    if surface == OverlaySurface::AccountMenu
        && let Some(index) = digit_shortcut_index(chord.logical_char()?)
    {
        return account_menu_action_at(index);
    }

    let ch = chord.logical_char()?;
    if surface == OverlaySurface::GitHubAuthModal
        && let Some(actions) = github_auth_modal_key_actions(state, ch)
    {
        return Some(actions);
    }

    if surface == OverlaySurface::Confirmation {
        return match ch.to_ascii_lowercase().as_str() {
            "y" => Some(vec![OverlayAction::ConfirmOverlaySelection.into()]),
            "n" => Some(vec![OverlayAction::CloseOverlay.into()]),
            _ => None,
        };
    }

    match ch {
        "j" => Some(vec![OverlayAction::MoveOverlaySelection(1).into()]),
        "k" => Some(vec![OverlayAction::MoveOverlaySelection(-1).into()]),
        "?" if surface == OverlaySurface::KeyboardShortcuts => {
            Some(vec![OverlayAction::ShowKeyboardShortcuts.into()])
        }
        _ => None,
    }
}

fn digit_shortcut_index(text: &str) -> Option<usize> {
    let mut chars = text.chars();
    let digit = chars.next()?.to_digit(10)?;
    if chars.next().is_some() || !(1..=9).contains(&digit) {
        return None;
    }
    Some(digit as usize - 1)
}

fn compare_menu_action_at(state: &AppState, index: usize) -> Option<Vec<Action>> {
    let profile = state.repository.location.with(&state.store, |location| {
        crate::ui::vcs::profile(location.as_ref())
    });
    let modes = profile.compare_modes();
    if let Some(mode) = modes.get(index) {
        return Some(vec![CompareAction::SetCompareMode(mode.mode).into()]);
    }

    let mut option_index = modes.len();
    if let Some(action) = branch_compare_preset_action(state, profile) {
        if index == option_index {
            return Some(vec![action]);
        }
        option_index += 1;
    }
    if let Some(action) = current_change_compare_preset_action(state, profile) {
        if index == option_index {
            return Some(vec![action]);
        }
        option_index += 1;
    }
    if let Some(action) = head_commit_compare_preset_action(state, profile)
        && index == option_index
    {
        return Some(vec![action]);
    }
    None
}

fn branch_compare_preset_action(
    state: &AppState,
    profile: crate::ui::vcs::VcsUiProfile,
) -> Option<Action> {
    let (head_branch, trunk) = state.repository.refs.with(&state.store, |refs| {
        let head = refs
            .iter()
            .find(|reference| reference.active && reference.kind == RefKind::Branch)
            .map(|reference| reference.name.clone());
        let trunk = refs
            .iter()
            .find(|reference| {
                reference.kind == RefKind::Branch
                    && matches!(reference.name.as_str(), "main" | "master" | "develop")
            })
            .map(|reference| reference.name.clone());
        (head, trunk)
    });
    if !profile.shows_branch_preset() {
        return None;
    }
    match (head_branch, trunk) {
        (Some(head), Some(trunk)) if head != trunk => {
            Some(CompareAction::ApplyComparePreset(format!("{trunk}:{head}:merge")).into())
        }
        _ => None,
    }
}

fn current_change_compare_preset_action(
    state: &AppState,
    profile: crate::ui::vcs::VcsUiProfile,
) -> Option<Action> {
    let current_change = state.repository.changes.with(&state.store, |changes| {
        changes
            .iter()
            .find(|change| change.flags.working_copy || change.flags.current)
            .cloned()
    });
    current_change
        .as_ref()
        .and_then(|change| profile.current_change_preset_label(change))
        .map(|_| CompareAction::ApplyComparePreset("@::commit".to_owned()).into())
}

fn head_commit_compare_preset_action(
    state: &AppState,
    profile: crate::ui::vcs::VcsUiProfile,
) -> Option<Action> {
    if !profile.shows_head_commit_preset() {
        return None;
    }
    state
        .repository
        .changes
        .with(&state.store, |changes| changes.first().cloned())
        .map(|commit| {
            CompareAction::ApplyComparePreset(format!("{}::commit", commit.revision.id)).into()
        })
}

fn publish_menu_action_at(state: &AppState, index: usize) -> Option<Vec<Action>> {
    let action = state.repository.publish_plan.with(&state.store, |plan| {
        let plan = plan.as_ref()?;
        if index == 0 {
            Some(plan.primary.clone())
        } else {
            plan.alternatives.get(index - 1).cloned()
        }
    })?;
    if !action.is_enabled() {
        return None;
    }
    Some(vec![RepositoryAction::Publish(action).into()])
}

fn account_menu_action_at(index: usize) -> Option<Vec<Action>> {
    let action: Action = match index {
        0 => SettingsAction::OpenSettings.into(),
        1 => GitHubAction::SignOutGitHub.into(),
        _ => return None,
    };
    Some(vec![action])
}

fn github_auth_modal_key_actions(state: &AppState, ch: &str) -> Option<Vec<Action>> {
    let flow = state.github.auth.device_flow.get(&state.store)?;
    match ch.to_ascii_lowercase().as_str() {
        "c" => Some(vec![AppAction::CopyText(flow.user_code).into()]),
        "o" => Some(vec![GitHubAction::OpenDeviceFlowBrowser.into()]),
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
            } else if state
                .editor
                .line_selection
                .with(&state.store, |selection| !selection.is_empty())
            {
                Some(vec![RepositoryAction::ClearLineSelection.into()])
            } else if state
                .workspace
                .pre_drill_compare
                .with(&state.store, |pre_drill| pre_drill.is_some())
            {
                Some(vec![CompareAction::ClearSidebarCommit.into()])
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
            if state.app_view.get(&state.store) == AppView::Settings {
                Some(vec![
                    SettingsAction::SetSettingsSection(adjacent_settings_section(state, 1)).into(),
                ])
            } else if state.focus.get(&state.store) == Some(FocusTarget::Editor) {
                Some(vec![EditorAction::ScrollViewportLines(1).into()])
            } else if state.is_workspace_ready() {
                Some(vec![FileListAction::SelectNextFile.into()])
            } else {
                None
            }
        }
        Some(NamedKey::ArrowUp) => {
            if state.app_view.get(&state.store) == AppView::Settings {
                Some(vec![
                    SettingsAction::SetSettingsSection(adjacent_settings_section(state, -1)).into(),
                ])
            } else if state.focus.get(&state.store) == Some(FocusTarget::Editor) {
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
            let binding = chord.binding_string()?;
            if state.app_view.get(&state.store) == AppView::Settings {
                return settings_key_actions(state, &binding);
            }
            if state.overlays_top().is_some()
                || state.workspace_mode.get(&state.store) != WorkspaceMode::Ready
            {
                return None;
            }
            let overrides = &state.settings.keymap_overrides;
            if matches_binding(overrides, ShortcutCommand::FocusSidebarSearch, &binding) {
                Some(vec![
                    AppAction::SetFocus(Some(FocusTarget::SidebarSearch)).into(),
                ])
            } else if matches_binding(overrides, ShortcutCommand::FocusFileList, &binding) {
                Some(vec![
                    AppAction::SetFocus(Some(FocusTarget::FileList)).into(),
                ])
            } else if matches_binding(overrides, ShortcutCommand::FocusEditor, &binding) {
                Some(vec![AppAction::SetFocus(Some(FocusTarget::Editor)).into()])
            } else if matches_binding(overrides, ShortcutCommand::NextHunk, &binding) {
                Some(vec![EditorAction::GoToNextHunk.into()])
            } else if matches_binding(overrides, ShortcutCommand::PreviousHunk, &binding) {
                Some(vec![EditorAction::GoToPreviousHunk.into()])
            } else if matches_binding(overrides, ShortcutCommand::NextFile, &binding) {
                Some(vec![EditorAction::GoToNextFile.into()])
            } else if matches_binding(overrides, ShortcutCommand::PreviousFile, &binding) {
                Some(vec![EditorAction::GoToPreviousFile.into()])
            } else if matches_binding(overrides, ShortcutCommand::MoveDown, &binding)
                && state.focus.get(&state.store) == Some(FocusTarget::FileList)
            {
                Some(vec![FileListAction::SelectNextFile.into()])
            } else if matches_binding(overrides, ShortcutCommand::MoveUp, &binding)
                && state.focus.get(&state.store) == Some(FocusTarget::FileList)
            {
                Some(vec![FileListAction::SelectPreviousFile.into()])
            } else if matches_binding(overrides, ShortcutCommand::MoveDown, &binding) {
                Some(vec![EditorAction::ScrollViewportLines(1).into()])
            } else if matches_binding(overrides, ShortcutCommand::MoveUp, &binding) {
                Some(vec![EditorAction::ScrollViewportLines(-1).into()])
            } else if matches_binding(overrides, ShortcutCommand::MoveRowDown, &binding) {
                Some(vec![EditorAction::MoveRowCursor(1).into()])
            } else if matches_binding(overrides, ShortcutCommand::MoveRowUp, &binding) {
                Some(vec![EditorAction::MoveRowCursor(-1).into()])
            } else if matches_binding(overrides, ShortcutCommand::ScrollHalfPageDown, &binding) {
                Some(vec![EditorAction::ScrollViewportHalfPage(1).into()])
            } else if matches_binding(overrides, ShortcutCommand::Unstage, &binding)
                && state.workspace.source.get(&state.store) == WorkspaceSource::Status
                && state.focus.get(&state.store) == Some(FocusTarget::FileList)
            {
                Some(vec![RepositoryAction::UnstageSelectedFile.into()])
            } else if matches_binding(overrides, ShortcutCommand::ScrollHalfPageUp, &binding) {
                Some(vec![EditorAction::ScrollViewportHalfPage(-1).into()])
            } else if matches_binding(overrides, ShortcutCommand::GoToBottom, &binding) {
                Some(vec![scroll_to_bottom_action(state).into()])
            } else if ch == "g" {
                let input = input.as_mut()?;
                if input.pending_g {
                    input.pending_g = false;
                    Some(vec![scroll_to_top_action(state).into()])
                } else {
                    input.pending_g = true;
                    Some(Vec::new())
                }
            } else if matches_binding(overrides, ShortcutCommand::UnifiedView, &binding) {
                Some(vec![
                    CompareAction::SetLayoutMode(crate::core::compare::LayoutMode::Unified).into(),
                ])
            } else if matches_binding(overrides, ShortcutCommand::SplitView, &binding) {
                Some(vec![
                    CompareAction::SetLayoutMode(crate::core::compare::LayoutMode::Split).into(),
                ])
            } else if matches_binding(overrides, ShortcutCommand::OpenCompareMenu, &binding) {
                Some(vec![CompareAction::OpenCompareMenu.into()])
            } else if matches_binding(overrides, ShortcutCommand::RefreshView, &binding) {
                Some(vec![WorkspaceAction::RefreshRepository.into()])
            } else if matches_binding(overrides, ShortcutCommand::ToggleFileTree, &binding) {
                Some(vec![FileListAction::ToggleSidebarMode.into()])
            } else if matches_binding(overrides, ShortcutCommand::ExpandFolders, &binding) {
                Some(vec![FileListAction::ExpandAllFolders.into()])
            } else if matches_binding(overrides, ShortcutCommand::CollapseFolders, &binding) {
                Some(vec![FileListAction::CollapseAllFolders.into()])
            } else if matches_binding(overrides, ShortcutCommand::SidebarFiles, &binding)
                && has_commit_tab(state)
            {
                Some(vec![
                    FileListAction::SetSidebarTab(SidebarTab::Files).into(),
                ])
            } else if matches_binding(overrides, ShortcutCommand::SidebarCommits, &binding)
                && has_commit_tab(state)
            {
                Some(vec![
                    FileListAction::SetSidebarTab(SidebarTab::Commits).into(),
                ])
            } else if matches_binding(overrides, ShortcutCommand::ToggleWrap, &binding) {
                Some(vec![SettingsAction::ToggleWrap.into()])
            } else if matches_binding(overrides, ShortcutCommand::ToggleLineSelection, &binding)
                && can_select_diff_lines(state)
            {
                Some(vec![RepositoryAction::ToggleCurrentLineSelection.into()])
            } else if matches_binding(
                overrides,
                ShortcutCommand::ToggleLineSelectionRange,
                &binding,
            ) && can_select_diff_lines(state)
            {
                Some(vec![
                    RepositoryAction::ToggleCurrentLineSelectionRange.into(),
                ])
            } else if matches_binding(overrides, ShortcutCommand::ReviewSelectedLines, &binding)
                && state
                    .editor
                    .line_selection
                    .with(&state.store, |selection| !selection.is_empty())
            {
                Some(vec![GitHubAction::OpenReviewCommentComposer.into()])
            } else if matches_binding(overrides, ShortcutCommand::Stage, &binding) {
                status_operation_actions(
                    state,
                    RepositoryAction::StageHunk,
                    RepositoryAction::StageSelectedLines,
                    RepositoryAction::StageSelectedFile,
                )
            } else if matches_binding(overrides, ShortcutCommand::Unstage, &binding) {
                status_operation_actions(
                    state,
                    RepositoryAction::UnstageHunk,
                    RepositoryAction::UnstageSelectedLines,
                    RepositoryAction::UnstageSelectedFile,
                )
            } else if matches_binding(overrides, ShortcutCommand::Discard, &binding) {
                status_operation_actions(
                    state,
                    RepositoryAction::DiscardHunk,
                    RepositoryAction::DiscardSelectedLines,
                    RepositoryAction::DiscardSelectedFile,
                )
            } else if matches_binding(overrides, ShortcutCommand::StageAll, &binding)
                && state.workspace.source.get(&state.store) == WorkspaceSource::Status
            {
                Some(vec![RepositoryAction::StageAllFiles.into()])
            } else if matches_binding(overrides, ShortcutCommand::UnstageAll, &binding)
                && state.workspace.source.get(&state.store) == WorkspaceSource::Status
            {
                Some(vec![RepositoryAction::UnstageAllFiles.into()])
            } else if matches_binding(overrides, ShortcutCommand::FocusCommitMessage, &binding)
                && state.workspace.source.get(&state.store) == WorkspaceSource::Status
            {
                Some(vec![
                    AppAction::SetFocus(Some(FocusTarget::CommitEditor)).into(),
                ])
            } else if matches_binding(overrides, ShortcutCommand::FetchRemotes, &binding) {
                Some(vec![RepositoryAction::FetchAllRemotes.into()])
            } else if matches_binding(overrides, ShortcutCommand::PullCurrentBranch, &binding) {
                Some(vec![RepositoryAction::PullCurrentBranch.into()])
            } else if matches_binding(overrides, ShortcutCommand::OpenPublishMenu, &binding) {
                Some(vec![RepositoryAction::OpenPublishMenu.into()])
            } else if matches_binding(overrides, ShortcutCommand::PageUp, &binding) {
                Some(vec![EditorAction::ScrollViewportPages(-1).into()])
            } else if matches_binding(overrides, ShortcutCommand::PageDown, &binding) {
                Some(vec![EditorAction::ScrollViewportPages(1).into()])
            } else {
                None
            }
        }
    }
}

fn settings_key_actions(state: &AppState, binding: &str) -> Option<Vec<Action>> {
    let overrides = &state.settings.keymap_overrides;
    if matches_binding(overrides, ShortcutCommand::SettingsNextSection, binding) {
        Some(vec![
            SettingsAction::SetSettingsSection(adjacent_settings_section(state, 1)).into(),
        ])
    } else if matches_binding(overrides, ShortcutCommand::SettingsPreviousSection, binding) {
        Some(vec![
            SettingsAction::SetSettingsSection(adjacent_settings_section(state, -1)).into(),
        ])
    } else if matches_binding(overrides, ShortcutCommand::SettingsAppearance, binding) {
        Some(vec![
            SettingsAction::SetSettingsSection(SettingsSection::Appearance).into(),
        ])
    } else if matches_binding(overrides, ShortcutCommand::SettingsEditor, binding) {
        Some(vec![
            SettingsAction::SetSettingsSection(SettingsSection::Editor).into(),
        ])
    } else if matches_binding(overrides, ShortcutCommand::SettingsBehavior, binding) {
        Some(vec![
            SettingsAction::SetSettingsSection(SettingsSection::Behavior).into(),
        ])
    } else if matches_binding(overrides, ShortcutCommand::SettingsKeymaps, binding) {
        Some(vec![
            SettingsAction::SetSettingsSection(SettingsSection::Keymaps).into(),
        ])
    } else if matches_binding(overrides, ShortcutCommand::SettingsClankers, binding) {
        Some(vec![
            SettingsAction::SetSettingsSection(SettingsSection::Clankers).into(),
        ])
    } else if matches_binding(overrides, ShortcutCommand::SettingsAbout, binding) {
        Some(vec![
            SettingsAction::SetSettingsSection(SettingsSection::About).into(),
        ])
    } else if matches_binding(overrides, ShortcutCommand::ToggleThemeMode, binding) {
        Some(vec![SettingsAction::ToggleThemeMode.into()])
    } else if matches_binding(overrides, ShortcutCommand::OpenThemePicker, binding) {
        Some(vec![SettingsAction::OpenThemePicker.into()])
    } else if matches_binding(overrides, ShortcutCommand::ToggleWrap, binding) {
        Some(vec![SettingsAction::ToggleWrap.into()])
    } else if matches_binding(overrides, ShortcutCommand::ToggleContinuousScroll, binding) {
        Some(vec![SettingsAction::ToggleContinuousScroll.into()])
    } else if matches_binding(overrides, ShortcutCommand::ToggleAutoUpdate, binding) {
        Some(vec![SettingsAction::ToggleAutoUpdate.into()])
    } else if matches_binding(overrides, ShortcutCommand::CheckUpdates, binding) {
        Some(vec![UpdateAction::CheckForUpdates.into()])
    } else {
        None
    }
}

fn adjacent_settings_section(state: &AppState, delta: i32) -> SettingsSection {
    let current = state.settings_section.get(&state.store);
    let sections = SettingsSection::ALL;
    let current_index = sections
        .iter()
        .position(|section| *section == current)
        .unwrap_or(0);
    let max = sections.len().saturating_sub(1) as i32;
    let next = (current_index as i32 + delta).clamp(0, max) as usize;
    sections[next]
}

fn has_commit_tab(state: &AppState) -> bool {
    state.workspace.source.get(&state.store) == WorkspaceSource::Compare
        && (state.file_list.tab.get(&state.store) == SidebarTab::Commits
            || state
                .workspace
                .range_commits
                .with(&state.store, |c| c.len())
                > 1
            || state
                .workspace
                .compare_history_pending
                .with(&state.store, |pending| pending.is_some()))
}

fn can_select_diff_lines(state: &AppState) -> bool {
    state.workspace.source.get(&state.store) == WorkspaceSource::Status
        && !state.settings.continuous_scroll
}

fn scroll_to_top_action(state: &AppState) -> EditorAction {
    if state.settings.continuous_scroll {
        EditorAction::ScrollViewportToGlobal(0)
    } else {
        EditorAction::ScrollViewportTo(0)
    }
}

fn scroll_to_bottom_action(state: &AppState) -> EditorAction {
    if state.settings.continuous_scroll {
        EditorAction::ScrollViewportToGlobal(state.global_max_scroll_top_px())
    } else {
        EditorAction::ScrollViewportTo(state.editor_max_scroll_top_px())
    }
}

fn status_operation_actions(
    state: &AppState,
    hunk_action: RepositoryAction,
    lines_action: RepositoryAction,
    file_action: RepositoryAction,
) -> Option<Vec<Action>> {
    if state.workspace.source.get(&state.store) != WorkspaceSource::Status {
        return None;
    }
    if state.focus.get(&state.store) == Some(FocusTarget::FileList) {
        return Some(vec![file_action.into()]);
    }
    if state
        .editor
        .line_selection
        .with(&state.store, |ls| ls.is_empty())
    {
        Some(vec![hunk_action.into()])
    } else {
        Some(vec![lines_action.into()])
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
        Some(OverlaySurface::ThemePicker | OverlaySurface::FontPicker) => {
            match state.focus.get(&state.store) {
                Some(FocusTarget::PickerInput) => Some(FocusTarget::PickerList),
                _ => Some(FocusTarget::PickerInput),
            }
        }
        Some(OverlaySurface::GitHubAuthModal) => Some(FocusTarget::AuthPrimaryAction),
        Some(
            OverlaySurface::KeyboardShortcuts
            | OverlaySurface::CompareMenu
            | OverlaySurface::AccountMenu
            | OverlaySurface::PublishMenu
            | OverlaySurface::Confirmation,
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
            | OverlaySurface::ThemePicker
            | OverlaySurface::FontPicker
            | OverlaySurface::Confirmation,
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

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use winit::keyboard::ModifiersState;

    use super::*;
    use crate::input::KeyKind;
    use crate::ui::editor::element::EditorElement;
    use crate::ui::editor::render_doc::{ByteRange, RenderDoc, RenderLine, RenderRowKind};
    use crate::ui::editor::state::{ViewportTextPoint, ViewportTextSelection, ViewportTextSide};
    use crate::ui::shell::UiFrame;
    use crate::ui::state::ViewportDocument;

    #[test]
    fn viewport_copy_shortcut_copies_current_text_selection() {
        let state = AppState::default();
        state.focus.set(&state.store, Some(FocusTarget::Editor));
        state.editor.text_selection.set(
            &state.store,
            Some(ViewportTextSelection {
                generation: 7,
                anchor: ViewportTextPoint {
                    line_index: 0,
                    side: ViewportTextSide::Right,
                    byte_offset: 1,
                },
                focus: ViewportTextPoint {
                    line_index: 0,
                    side: ViewportTextSide::Right,
                    byte_offset: 4,
                },
            }),
        );
        let doc = Arc::new(RenderDoc {
            file_metadata: Vec::new(),
            text_bytes: b"alpha".to_vec(),
            style_runs: Vec::new(),
            lines: vec![RenderLine {
                kind: RenderRowKind::Context as u8,
                old_line_no: 1,
                new_line_no: 1,
                right_text: ByteRange { start: 0, len: 5 },
                right_cols: 5,
                ..RenderLine::default()
            }],
        });
        let ui_frame = UiFrame {
            viewport_document: Some(ViewportDocument::single(doc, 7, 0, "demo.txt".to_owned())),
            ..UiFrame::default()
        };
        let chord = KeyChord {
            logical: KeyKind::Character("c".to_owned()),
            physical: None,
            modifiers: ModifiersState::CONTROL,
            repeat: false,
        };
        let ctx = InputContext {
            owner: InputOwner::Editor,
            overlay: None,
            focus: Some(FocusTarget::Editor),
            workspace_mode: WorkspaceMode::Ready,
            ime_active: false,
        };

        let actions = viewport_clipboard_shortcut_actions(
            &state,
            &ui_frame,
            &EditorElement::default(),
            &ctx,
            &chord,
        )
        .expect("copy action");

        assert_eq!(actions, vec![AppAction::CopyText("lph".to_owned()).into()]);
    }
}

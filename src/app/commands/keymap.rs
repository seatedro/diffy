use quark_app::keymap::{self, Keymap, ShortcutEntry, ShortcutGroup, ShortcutScope};
use quark_ui::key_context::KeyBindings;

use crate::contracts::commands::{CommandsAction, SettingsSection};
use crate::contracts::delivery::DeliveryAction;
use crate::contracts::mutations::MutationsAction;
use crate::contracts::navigation::{NavigationAction, SidebarTab};
use crate::contracts::shell::{Action, LogicalFocus, ShellAction};
use crate::contracts::viewport::{InteractionAction, ViewportAction};
use crate::core::compare::LayoutMode;
use crate::core::keymap::{KeymapOverride, ShortcutCommand};

pub type Override = keymap::KeymapOverride<ShortcutCommand>;
pub type Entry = ShortcutEntry<ShortcutCommand>;

pub const WORKSPACE_CONTEXT: &str = "surface=workspace";
pub const SETTINGS_CONTEXT: &str = "surface=settings";
pub const TEXT_CONTEXT: &str = "surface=text";

const WORKSPACE: ShortcutScope = ShortcutScope("surface == workspace");
const SETTINGS: ShortcutScope = ShortcutScope("surface == settings");
const TEXT: ShortcutScope = ShortcutScope("surface == text");
const GLOBAL: ShortcutScope = ShortcutScope::GLOBAL;

use ShortcutCommand as C;

const NAVIGATION: &[Entry] = &[
    Entry::new(WORKSPACE, C::NextHunk, &["]"], "Next hunk"),
    Entry::new(WORKSPACE, C::PreviousHunk, &["["], "Previous hunk"),
    Entry::new(WORKSPACE, C::FocusFileList, &["h"], "Focus file list"),
    Entry::new(WORKSPACE, C::FocusEditor, &["l"], "Focus diff"),
    Entry::new(
        WORKSPACE,
        C::MoveDown,
        &["j"],
        "Move selection or scroll down",
    ),
    Entry::new(WORKSPACE, C::MoveUp, &["k"], "Move selection or scroll up"),
    Entry::new(
        WORKSPACE,
        C::MoveRowDown,
        &["shift+j"],
        "Move diff row cursor down",
    ),
    Entry::new(
        WORKSPACE,
        C::MoveRowUp,
        &["shift+k"],
        "Move diff row cursor up",
    ),
    Entry::new(WORKSPACE, C::NextFile, &["n"], "Next file"),
    Entry::new(WORKSPACE, C::PreviousFile, &["shift+n"], "Previous file"),
    Entry::new(
        WORKSPACE,
        C::ToggleFocus,
        &["tab"],
        "Toggle sidebar / editor focus",
    ),
    Entry::new(
        WORKSPACE,
        C::FocusSidebarSearch,
        &["/"],
        "Focus sidebar search",
    ),
    Entry::new(
        WORKSPACE,
        C::SidebarFiles,
        &["shift+f"],
        "Files sidebar tab",
    ),
    Entry::new(
        WORKSPACE,
        C::SidebarCommits,
        &["shift+c"],
        "History sidebar tab",
    ),
];

const SCROLLING: &[Entry] = &[
    Entry::new(
        WORKSPACE,
        C::ScrollHalfPageDown,
        &["d"],
        "Scroll down half page",
    ),
    Entry::new(
        WORKSPACE,
        C::ScrollHalfPageUp,
        &["u"],
        "Scroll up half page",
    ),
    Entry::new(WORKSPACE, C::PageDown, &["space"], "Page down"),
    Entry::new(WORKSPACE, C::PageUp, &["shift+space"], "Page up"),
    Entry::new(WORKSPACE, C::GoToBottom, &["shift+g"], "Go to bottom"),
];

const VIEW: &[Entry] = &[
    Entry::new(WORKSPACE, C::UnifiedView, &["1"], "Unified diff view"),
    Entry::new(WORKSPACE, C::SplitView, &["2"], "Split diff view"),
    Entry::new(WORKSPACE, C::OpenCompareMenu, &["m"], "Open compare menu"),
    Entry::new(WORKSPACE, C::RefreshView, &["r"], "Refresh current view"),
    Entry::new(WORKSPACE, C::ToggleFileTree, &["t"], "Toggle file tree"),
    Entry::new(
        WORKSPACE,
        C::ExpandFolders,
        &["=", "shift+="],
        "Expand folders",
    ),
    Entry::new(WORKSPACE, C::CollapseFolders, &["-"], "Collapse folders"),
    Entry::new(WORKSPACE, C::ToggleWrap, &["w"], "Toggle line wrapping"),
    Entry::new(GLOBAL, C::ToggleSidebar, &["mod+b"], "Toggle sidebar"),
];

const SEARCH: &[Entry] = &[
    Entry::new(GLOBAL, C::OpenSearch, &["mod+f"], "Open search"),
    Entry::new(
        GLOBAL,
        C::OpenCommandPalette,
        &["mod+p", "mod+k"],
        "Command palette",
    ),
    Entry::new(GLOBAL, C::ShowKeymaps, &["shift+/"], "Open keymaps"),
];

const SETTINGS_ENTRIES: &[Entry] = &[
    Entry::new(GLOBAL, C::OpenSettings, &["mod+,"], "Open settings"),
    Entry::new(
        GLOBAL,
        C::IncreaseUiScale,
        &["mod+=", "mod+shift+="],
        "Increase UI scale",
    ),
    Entry::new(GLOBAL, C::DecreaseUiScale, &["mod+-"], "Decrease UI scale"),
    Entry::new(
        SETTINGS,
        C::SettingsAppearance,
        &["1"],
        "Settings: Appearance",
    ),
    Entry::new(SETTINGS, C::SettingsEditor, &["2"], "Settings: Editor"),
    Entry::new(SETTINGS, C::SettingsBehavior, &["3"], "Settings: Behavior"),
    Entry::new(SETTINGS, C::SettingsKeymaps, &["4"], "Settings: Keymaps"),
    Entry::new(SETTINGS, C::SettingsClankers, &["5"], "Settings: Clankers"),
    Entry::new(SETTINGS, C::SettingsAbout, &["6"], "Settings: About"),
    Entry::new(
        SETTINGS,
        C::SettingsNextSection,
        &["j"],
        "Next settings section",
    ),
    Entry::new(
        SETTINGS,
        C::SettingsPreviousSection,
        &["k"],
        "Previous settings section",
    ),
    Entry::new(SETTINGS, C::ToggleThemeMode, &["t"], "Toggle theme mode"),
    Entry::new(SETTINGS, C::OpenThemePicker, &["b"], "Browse themes"),
    Entry::new(SETTINGS, C::ToggleWrap, &["w"], "Toggle wrap"),
    Entry::new(
        SETTINGS,
        C::ToggleContinuousScroll,
        &["c"],
        "Toggle continuous scroll",
    ),
    Entry::new(SETTINGS, C::ToggleAutoUpdate, &["a"], "Toggle auto-update"),
    Entry::new(SETTINGS, C::CheckUpdates, &["u"], "Check updates"),
];

const WORKING_TREE: &[Entry] = &[
    Entry::new(
        WORKSPACE,
        C::Stage,
        &["s"],
        "Stage file / hunk / selected lines",
    ),
    Entry::new(
        WORKSPACE,
        C::Unstage,
        &["shift+s", "shift+u"],
        "Unstage file / hunk / selected lines",
    ),
    Entry::new(
        WORKSPACE,
        C::Discard,
        &["x"],
        "Discard file / hunk / selected lines",
    ),
    Entry::new(WORKSPACE, C::StageAll, &["a"], "Stage all"),
    Entry::new(WORKSPACE, C::UnstageAll, &["shift+a"], "Unstage all"),
    Entry::new(
        WORKSPACE,
        C::FocusCommitMessage,
        &["c"],
        "Focus commit message",
    ),
    Entry::new(
        TEXT,
        C::SubmitCommit,
        &["mod+enter"],
        "Create commit from message",
    ),
    Entry::new(
        WORKSPACE,
        C::ToggleLineSelection,
        &["v"],
        "Select changed line",
    ),
    Entry::new(
        WORKSPACE,
        C::ToggleLineSelectionRange,
        &["shift+v"],
        "Select changed line range",
    ),
    Entry::new(
        WORKSPACE,
        C::ReviewSelectedLines,
        &["shift+r"],
        "Comment on selected lines",
    ),
];

const REPOSITORY: &[Entry] = &[
    Entry::new(WORKSPACE, C::FetchRemotes, &["f"], "Fetch remotes"),
    Entry::new(
        WORKSPACE,
        C::PullCurrentBranch,
        &["p"],
        "Pull current branch",
    ),
    Entry::new(
        WORKSPACE,
        C::OpenPublishMenu,
        &["shift+p"],
        "Publish options",
    ),
];

const DEBUG: &[Entry] = &[Entry::new(
    GLOBAL,
    C::ToggleDebugOverlay,
    &["mod+shift+d"],
    "Toggle debug overlay",
)];

const GROUPS: &[ShortcutGroup<ShortcutCommand>] = &[
    ShortcutGroup {
        title: "Navigation",
        entries: NAVIGATION,
    },
    ShortcutGroup {
        title: "Scrolling",
        entries: SCROLLING,
    },
    ShortcutGroup {
        title: "View",
        entries: VIEW,
    },
    ShortcutGroup {
        title: "Search",
        entries: SEARCH,
    },
    ShortcutGroup {
        title: "Settings",
        entries: SETTINGS_ENTRIES,
    },
    ShortcutGroup {
        title: "Working Tree",
        entries: WORKING_TREE,
    },
    ShortcutGroup {
        title: "Repository",
        entries: REPOSITORY,
    },
    ShortcutGroup {
        title: "Debug",
        entries: DEBUG,
    },
];

pub const KEYMAP: Keymap<ShortcutCommand> = Keymap::new(GROUPS);

pub fn overrides(saved: &[KeymapOverride]) -> Vec<Override> {
    saved
        .iter()
        .map(|o| Override {
            command: o.command,
            binding: o.binding.clone(),
        })
        .collect()
}

pub fn key_bindings(saved: &[KeymapOverride]) -> KeyBindings {
    KEYMAP.key_bindings(&overrides(saved), |command| {
        Action::from(CommandsAction::Shortcut(command)).into()
    })
}

pub fn display_binding(saved: &[KeymapOverride], command: ShortcutCommand) -> Option<String> {
    KEYMAP
        .active_bindings(&overrides(saved), command)
        .first()
        .map(|binding| keymap::format_binding(binding))
}

pub fn first_binding(saved: &[KeymapOverride], command: ShortcutCommand) -> Option<String> {
    KEYMAP
        .active_bindings(&overrides(saved), command)
        .first()
        .map(|binding| (*binding).to_owned())
}

pub fn conflict(
    saved: &[KeymapOverride],
    command: ShortcutCommand,
    binding: &str,
) -> Option<&'static Entry> {
    let entry = KEYMAP.entry(command)?;
    KEYMAP.binding_conflict(&overrides(saved), entry, binding)
}

pub fn description(command: ShortcutCommand) -> &'static str {
    KEYMAP
        .entry(command)
        .map(|entry| entry.description)
        .unwrap_or_default()
}

fn interaction(action: InteractionAction) -> Action {
    Action::Viewport(ViewportAction::Interaction(action))
}

fn section(section: SettingsSection) -> Action {
    CommandsAction::SetSettingsSection(section).into()
}

pub fn builtin_action(command: ShortcutCommand) -> Option<Action> {
    Some(match command {
        C::OpenSearch => interaction(InteractionAction::OpenSearch),
        C::OpenCommandPalette => CommandsAction::OpenCommandPalette.into(),
        C::IncreaseUiScale => CommandsAction::IncreaseUiScale.into(),
        C::DecreaseUiScale => CommandsAction::DecreaseUiScale.into(),
        C::ToggleSidebar => NavigationAction::ToggleSidebar.into(),
        C::OpenSettings => CommandsAction::OpenSettings.into(),
        C::ShowKeymaps => CommandsAction::OpenKeymaps.into(),
        C::NextHunk => interaction(InteractionAction::GoToNextHunk),
        C::PreviousHunk => interaction(InteractionAction::GoToPreviousHunk),
        C::FocusFileList => ShellAction::FocusLogical(Some(LogicalFocus::Sidebar)).into(),
        C::FocusEditor => ShellAction::FocusLogical(Some(LogicalFocus::Viewport)).into(),
        C::MoveDown => NavigationAction::SelectNextFile.into(),
        C::MoveUp => NavigationAction::SelectPreviousFile.into(),
        C::MoveRowDown => interaction(InteractionAction::MoveRowCursor(1)),
        C::MoveRowUp => interaction(InteractionAction::MoveRowCursor(-1)),
        C::NextFile => NavigationAction::SelectNextFile.into(),
        C::PreviousFile => NavigationAction::SelectPreviousFile.into(),
        C::FocusSidebarSearch => {
            ShellAction::FocusLogical(Some(LogicalFocus::SidebarFilter)).into()
        }
        C::SidebarFiles => NavigationAction::SetSidebarTab(SidebarTab::Files).into(),
        C::SidebarCommits => NavigationAction::SetSidebarTab(SidebarTab::Commits).into(),
        C::ScrollHalfPageDown => interaction(InteractionAction::ScrollHalfPage(1)),
        C::ScrollHalfPageUp => interaction(InteractionAction::ScrollHalfPage(-1)),
        C::PageDown => interaction(InteractionAction::ScrollPages(1)),
        C::PageUp => interaction(InteractionAction::ScrollPages(-1)),
        C::GoToBottom => interaction(InteractionAction::ScrollToBottom),
        C::UnifiedView => CommandsAction::SetLayout(LayoutMode::Unified).into(),
        C::SplitView => CommandsAction::SetLayout(LayoutMode::Split).into(),
        C::OpenCompareMenu => CommandsAction::OpenCompareMenu.into(),
        C::RefreshView => NavigationAction::RefreshRepository.into(),
        C::ToggleFileTree => NavigationAction::ToggleSidebarMode.into(),
        C::ExpandFolders => NavigationAction::ExpandAllFolders.into(),
        C::CollapseFolders => NavigationAction::CollapseAllFolders.into(),
        C::ToggleWrap => CommandsAction::ToggleWrap.into(),
        C::SettingsAppearance => section(SettingsSection::Appearance),
        C::SettingsEditor => section(SettingsSection::Editor),
        C::SettingsBehavior => section(SettingsSection::Behavior),
        C::SettingsKeymaps => section(SettingsSection::Keymaps),
        C::SettingsClankers => section(SettingsSection::Clankers),
        C::SettingsAbout => section(SettingsSection::Updates),
        C::ToggleThemeMode => CommandsAction::ToggleThemeMode.into(),
        C::OpenThemePicker => CommandsAction::OpenThemePicker.into(),
        C::ToggleContinuousScroll => CommandsAction::ToggleContinuousScroll.into(),
        C::ToggleAutoUpdate => CommandsAction::ToggleAutoUpdate.into(),
        C::CheckUpdates => DeliveryAction::CheckForUpdates.into(),
        C::StageAll => MutationsAction::StageAllFiles.into(),
        C::UnstageAll => MutationsAction::UnstageAllFiles.into(),
        C::FocusCommitMessage => {
            ShellAction::FocusLogical(Some(LogicalFocus::CommitMessage)).into()
        }
        C::SubmitCommit => MutationsAction::SubmitCommit.into(),
        C::ToggleLineSelection => MutationsAction::ToggleCurrentLineSelection.into(),
        C::ToggleLineSelectionRange => MutationsAction::ToggleCurrentLineSelectionRange.into(),
        C::FetchRemotes => MutationsAction::FetchAllRemotes.into(),
        C::PullCurrentBranch => MutationsAction::PullCurrentBranch.into(),
        C::OpenPublishMenu => MutationsAction::OpenPublishMenu.into(),
        C::ToggleDebugOverlay => ShellAction::ToggleDebugOverlay.into(),
        C::ToggleFocus
        | C::SettingsNextSection
        | C::SettingsPreviousSection
        | C::Stage
        | C::Unstage
        | C::Discard
        | C::ReviewSelectedLines
        | C::ConfirmOverlay
        | C::CloseOverlay => return None,
    })
}

pub fn settings_section_step(current: SettingsSection, delta: i32) -> SettingsSection {
    let all = SettingsSection::ALL;
    let index = all.iter().position(|s| *s == current).unwrap_or(0) as i32;
    all[(index + delta).rem_euclid(all.len() as i32) as usize]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_keymap_has_no_contextual_conflicts() {
        for entry in KEYMAP.entries() {
            for binding in entry.keys {
                assert!(
                    KEYMAP.binding_conflict(&[], entry, binding).is_none(),
                    "{} conflicts on {binding}",
                    entry.description
                );
            }
        }
    }

    #[test]
    fn overrides_replace_defaults_and_report_conflicts() {
        let saved = vec![KeymapOverride {
            command: C::ToggleWrap,
            binding: "shift+w".to_owned(),
        }];
        assert_eq!(
            first_binding(&saved, C::ToggleWrap).as_deref(),
            Some("shift+w")
        );
        assert_eq!(
            conflict(&saved, C::NextFile, "shift+w").map(|e| e.command),
            Some(C::ToggleWrap)
        );
        assert!(conflict(&saved, C::NextFile, "w").is_none());
        assert_eq!(
            conflict(&[], C::OpenSettings, "mod+b").map(|e| e.command),
            Some(C::ToggleSidebar)
        );
        assert_eq!(
            conflict(&[], C::SettingsAppearance, "2").map(|e| e.command),
            Some(C::SettingsEditor)
        );
        assert!(conflict(&[], C::UnifiedView, "3").is_none());
    }
}

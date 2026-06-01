use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ShortcutCommand {
    OpenSearch,
    OpenCommandPalette,
    IncreaseUiScale,
    DecreaseUiScale,
    ToggleSidebar,
    OpenSettings,
    ShowKeymaps,
    NextHunk,
    PreviousHunk,
    FocusFileList,
    FocusEditor,
    MoveDown,
    MoveUp,
    MoveRowDown,
    MoveRowUp,
    NextFile,
    PreviousFile,
    ToggleFocus,
    FocusSidebarSearch,
    SidebarFiles,
    SidebarCommits,
    ScrollHalfPageDown,
    ScrollHalfPageUp,
    PageDown,
    PageUp,
    GoToBottom,
    UnifiedView,
    SplitView,
    OpenCompareMenu,
    RefreshView,
    ToggleFileTree,
    ExpandFolders,
    CollapseFolders,
    ToggleWrap,
    SettingsAppearance,
    SettingsEditor,
    SettingsBehavior,
    SettingsKeymaps,
    SettingsClankers,
    SettingsAbout,
    SettingsNextSection,
    SettingsPreviousSection,
    ToggleThemeMode,
    OpenThemePicker,
    ToggleContinuousScroll,
    ToggleAutoUpdate,
    CheckUpdates,
    Stage,
    Unstage,
    Discard,
    StageAll,
    UnstageAll,
    FocusCommitMessage,
    SubmitCommit,
    ToggleLineSelection,
    ToggleLineSelectionRange,
    ReviewSelectedLines,
    FetchRemotes,
    PullCurrentBranch,
    OpenPublishMenu,
    ConfirmOverlay,
    CloseOverlay,
    ToggleDebugOverlay,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeymapOverride {
    pub command: ShortcutCommand,
    pub binding: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShortcutScope {
    Global,
    Workspace,
    Settings,
    TextField,
}

impl ShortcutScope {
    fn overlaps(self, other: Self) -> bool {
        self == other || self == Self::Global || other == Self::Global
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShortcutEntry {
    pub command: ShortcutCommand,
    pub scope: ShortcutScope,
    pub keys: &'static [&'static str],
    pub description: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShortcutGroup {
    pub title: &'static str,
    pub entries: &'static [ShortcutEntry],
}

const NAVIGATION: &[ShortcutEntry] = &[
    workspace_entry(ShortcutCommand::NextHunk, &["]"], "Next hunk"),
    workspace_entry(ShortcutCommand::PreviousHunk, &["["], "Previous hunk"),
    workspace_entry(ShortcutCommand::FocusFileList, &["h"], "Focus file list"),
    workspace_entry(ShortcutCommand::FocusEditor, &["l"], "Focus diff"),
    workspace_entry(
        ShortcutCommand::MoveDown,
        &["j"],
        "Move selection or scroll down",
    ),
    workspace_entry(
        ShortcutCommand::MoveUp,
        &["k"],
        "Move selection or scroll up",
    ),
    workspace_entry(
        ShortcutCommand::MoveRowDown,
        &["shift+j"],
        "Move diff row cursor down",
    ),
    workspace_entry(
        ShortcutCommand::MoveRowUp,
        &["shift+k"],
        "Move diff row cursor up",
    ),
    workspace_entry(ShortcutCommand::NextFile, &["n"], "Next file"),
    workspace_entry(ShortcutCommand::PreviousFile, &["shift+n"], "Previous file"),
    workspace_entry(
        ShortcutCommand::ToggleFocus,
        &["tab"],
        "Toggle sidebar / editor focus",
    ),
    workspace_entry(
        ShortcutCommand::FocusSidebarSearch,
        &["/"],
        "Focus sidebar search",
    ),
    workspace_entry(
        ShortcutCommand::SidebarFiles,
        &["shift+f"],
        "Files sidebar tab",
    ),
    workspace_entry(
        ShortcutCommand::SidebarCommits,
        &["shift+c"],
        "Commits sidebar tab",
    ),
];

const SCROLLING: &[ShortcutEntry] = &[
    workspace_entry(
        ShortcutCommand::ScrollHalfPageDown,
        &["d"],
        "Scroll down half page",
    ),
    workspace_entry(
        ShortcutCommand::ScrollHalfPageUp,
        &["u"],
        "Scroll up half page",
    ),
    workspace_entry(ShortcutCommand::PageDown, &["space"], "Page down"),
    workspace_entry(ShortcutCommand::PageUp, &["shift+space"], "Page up"),
    workspace_entry(ShortcutCommand::GoToBottom, &["shift+g"], "Go to bottom"),
];

const VIEW: &[ShortcutEntry] = &[
    workspace_entry(ShortcutCommand::UnifiedView, &["1"], "Unified diff view"),
    workspace_entry(ShortcutCommand::SplitView, &["2"], "Split diff view"),
    workspace_entry(
        ShortcutCommand::OpenCompareMenu,
        &["m"],
        "Open compare menu",
    ),
    workspace_entry(ShortcutCommand::RefreshView, &["r"], "Refresh current view"),
    workspace_entry(ShortcutCommand::ToggleFileTree, &["t"], "Toggle file tree"),
    workspace_entry(
        ShortcutCommand::ExpandFolders,
        &["=", "shift+="],
        "Expand folders",
    ),
    workspace_entry(ShortcutCommand::CollapseFolders, &["-"], "Collapse folders"),
    workspace_entry(ShortcutCommand::ToggleWrap, &["w"], "Toggle line wrapping"),
    global_entry(ShortcutCommand::ToggleSidebar, &["mod+b"], "Toggle sidebar"),
];

const SEARCH: &[ShortcutEntry] = &[
    global_entry(ShortcutCommand::OpenSearch, &["mod+f"], "Open search"),
    global_entry(
        ShortcutCommand::OpenCommandPalette,
        &["mod+p", "mod+k"],
        "Command palette",
    ),
    global_entry(ShortcutCommand::ShowKeymaps, &["shift+/"], "Open keymaps"),
];

const SETTINGS: &[ShortcutEntry] = &[
    global_entry(ShortcutCommand::OpenSettings, &["mod+,"], "Open settings"),
    global_entry(
        ShortcutCommand::IncreaseUiScale,
        &["mod+=", "mod+shift+="],
        "Increase UI scale",
    ),
    global_entry(
        ShortcutCommand::DecreaseUiScale,
        &["mod+-"],
        "Decrease UI scale",
    ),
    settings_entry(
        ShortcutCommand::SettingsAppearance,
        &["1"],
        "Settings: Appearance",
    ),
    settings_entry(ShortcutCommand::SettingsEditor, &["2"], "Settings: Editor"),
    settings_entry(
        ShortcutCommand::SettingsBehavior,
        &["3"],
        "Settings: Behavior",
    ),
    settings_entry(
        ShortcutCommand::SettingsKeymaps,
        &["4"],
        "Settings: Keymaps",
    ),
    settings_entry(
        ShortcutCommand::SettingsClankers,
        &["5"],
        "Settings: Clankers",
    ),
    settings_entry(ShortcutCommand::SettingsAbout, &["6"], "Settings: About"),
    settings_entry(
        ShortcutCommand::SettingsNextSection,
        &["j"],
        "Next settings section",
    ),
    settings_entry(
        ShortcutCommand::SettingsPreviousSection,
        &["k"],
        "Previous settings section",
    ),
    settings_entry(
        ShortcutCommand::ToggleThemeMode,
        &["t"],
        "Toggle theme mode",
    ),
    settings_entry(ShortcutCommand::OpenThemePicker, &["b"], "Browse themes"),
    settings_entry(ShortcutCommand::ToggleWrap, &["w"], "Toggle wrap"),
    settings_entry(
        ShortcutCommand::ToggleContinuousScroll,
        &["c"],
        "Toggle continuous scroll",
    ),
    settings_entry(
        ShortcutCommand::ToggleAutoUpdate,
        &["a"],
        "Toggle auto-update",
    ),
    settings_entry(ShortcutCommand::CheckUpdates, &["u"], "Check updates"),
];

const WORKING_TREE: &[ShortcutEntry] = &[
    workspace_entry(
        ShortcutCommand::Stage,
        &["s"],
        "Stage file / hunk / selected lines",
    ),
    workspace_entry(
        ShortcutCommand::Unstage,
        &["shift+s", "shift+u"],
        "Unstage file / hunk / selected lines",
    ),
    workspace_entry(
        ShortcutCommand::Discard,
        &["x"],
        "Discard file / hunk / selected lines",
    ),
    workspace_entry(ShortcutCommand::StageAll, &["a"], "Stage all"),
    workspace_entry(ShortcutCommand::UnstageAll, &["shift+a"], "Unstage all"),
    workspace_entry(
        ShortcutCommand::FocusCommitMessage,
        &["c"],
        "Focus commit message",
    ),
    text_field_entry(
        ShortcutCommand::SubmitCommit,
        &["mod+enter"],
        "Create commit from message",
    ),
    workspace_entry(
        ShortcutCommand::ToggleLineSelection,
        &["v"],
        "Select changed line",
    ),
    workspace_entry(
        ShortcutCommand::ToggleLineSelectionRange,
        &["shift+v"],
        "Select changed line range",
    ),
    workspace_entry(
        ShortcutCommand::ReviewSelectedLines,
        &["shift+r"],
        "Comment on selected lines",
    ),
];

const REPOSITORY: &[ShortcutEntry] = &[
    workspace_entry(ShortcutCommand::FetchRemotes, &["f"], "Fetch remotes"),
    workspace_entry(
        ShortcutCommand::PullCurrentBranch,
        &["p"],
        "Pull current branch",
    ),
    workspace_entry(
        ShortcutCommand::OpenPublishMenu,
        &["shift+p"],
        "Publish options",
    ),
];

const DEBUG: &[ShortcutEntry] = &[global_entry(
    ShortcutCommand::ToggleDebugOverlay,
    &["mod+shift+d"],
    "Toggle debug overlay",
)];

const GROUPS: &[ShortcutGroup] = &[
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
        entries: SETTINGS,
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

const fn global_entry(
    command: ShortcutCommand,
    keys: &'static [&'static str],
    description: &'static str,
) -> ShortcutEntry {
    entry(ShortcutScope::Global, command, keys, description)
}

const fn workspace_entry(
    command: ShortcutCommand,
    keys: &'static [&'static str],
    description: &'static str,
) -> ShortcutEntry {
    entry(ShortcutScope::Workspace, command, keys, description)
}

const fn settings_entry(
    command: ShortcutCommand,
    keys: &'static [&'static str],
    description: &'static str,
) -> ShortcutEntry {
    entry(ShortcutScope::Settings, command, keys, description)
}

const fn text_field_entry(
    command: ShortcutCommand,
    keys: &'static [&'static str],
    description: &'static str,
) -> ShortcutEntry {
    entry(ShortcutScope::TextField, command, keys, description)
}

const fn entry(
    scope: ShortcutScope,
    command: ShortcutCommand,
    keys: &'static [&'static str],
    description: &'static str,
) -> ShortcutEntry {
    ShortcutEntry {
        command,
        scope,
        keys,
        description,
    }
}

pub fn shortcut_groups() -> &'static [ShortcutGroup] {
    GROUPS
}

pub fn shortcut_entries() -> impl Iterator<Item = &'static ShortcutEntry> {
    GROUPS.iter().flat_map(|group| group.entries.iter())
}

pub fn shortcut_entry(command: ShortcutCommand) -> Option<&'static ShortcutEntry> {
    shortcut_entries().find(|entry| entry.command == command)
}

pub fn override_for(
    overrides: &[KeymapOverride],
    command: ShortcutCommand,
) -> Option<&KeymapOverride> {
    overrides.iter().find(|binding| binding.command == command)
}

pub fn active_bindings(overrides: &[KeymapOverride], command: ShortcutCommand) -> Vec<&str> {
    if let Some(binding) = override_for(overrides, command) {
        return vec![binding.binding.as_str()];
    }
    shortcut_entry(command)
        .map(|entry| entry.keys.to_vec())
        .unwrap_or_default()
}

pub fn binding_matches(
    overrides: &[KeymapOverride],
    command: ShortcutCommand,
    binding: &str,
) -> bool {
    active_bindings(overrides, command)
        .iter()
        .any(|candidate| binding_eq(candidate, binding))
}

pub fn binding_conflict(
    overrides: &[KeymapOverride],
    entry: &ShortcutEntry,
    binding: &str,
) -> Option<&'static ShortcutEntry> {
    shortcut_entries().find(|candidate| {
        candidate.command != entry.command
            && candidate.scope.overlaps(entry.scope)
            && binding_matches(overrides, candidate.command, binding)
    })
}

pub fn set_override(
    overrides: &mut Vec<KeymapOverride>,
    command: ShortcutCommand,
    binding: String,
) {
    if let Some(existing) = overrides
        .iter_mut()
        .find(|existing| existing.command == command)
    {
        existing.binding = binding;
    } else {
        overrides.push(KeymapOverride { command, binding });
    }
}

pub fn reset_override(overrides: &mut Vec<KeymapOverride>, command: ShortcutCommand) {
    overrides.retain(|binding| binding.command != command);
}

pub fn binding_eq(left: &str, right: &str) -> bool {
    if left.eq_ignore_ascii_case(right) {
        return true;
    }
    let left_parts = left.split('+').collect::<Vec<_>>();
    let right_parts = right.split('+').collect::<Vec<_>>();
    left_parts.len() == right_parts.len()
        && left_parts
            .iter()
            .zip(right_parts.iter())
            .all(|(left, right)| binding_part_eq(left, right))
}

fn binding_part_eq(left: &str, right: &str) -> bool {
    left.eq_ignore_ascii_case(right)
        || (left == "mod" && matches!(right, "cmd" | "ctrl"))
        || (right == "mod" && matches!(left, "cmd" | "ctrl"))
}

pub fn format_binding(binding: &str) -> String {
    if binding.contains(' ') {
        return binding
            .split_whitespace()
            .map(format_binding)
            .collect::<Vec<_>>()
            .join(" then ");
    }
    if let Some(display) = display_shifted_symbol(binding) {
        return display.to_owned();
    }

    let mut parts = binding.split('+').collect::<Vec<_>>();
    let Some(key) = parts.pop() else {
        return binding.to_owned();
    };
    let modifiers = parts
        .into_iter()
        .map(|part| match part {
            "mod" if cfg!(target_os = "macos") => "Cmd".to_owned(),
            "mod" => "Ctrl".to_owned(),
            "cmd" => "Cmd".to_owned(),
            "ctrl" => "Ctrl".to_owned(),
            "alt" => "Alt".to_owned(),
            "shift" => "Shift".to_owned(),
            other => title_case(other),
        })
        .collect::<Vec<_>>();

    if modifiers.is_empty() {
        match key {
            "shift+/" => "?".to_owned(),
            _ => display_key(key),
        }
    } else {
        let key = display_key(key);
        [modifiers, vec![key]].concat().join("+")
    }
}

fn display_key(key: &str) -> String {
    match key {
        "escape" => "Esc".to_owned(),
        "enter" => "Enter".to_owned(),
        "tab" => "Tab".to_owned(),
        "space" => "Space".to_owned(),
        "arrowup" => "Up".to_owned(),
        "arrowdown" => "Down".to_owned(),
        "arrowleft" => "Left".to_owned(),
        "arrowright" => "Right".to_owned(),
        "pagedown" => "Page Down".to_owned(),
        "pageup" => "Page Up".to_owned(),
        key if key.len() == 1 => key.to_ascii_uppercase(),
        key => title_case(key),
    }
}

fn display_shifted_symbol(binding: &str) -> Option<&'static str> {
    match binding {
        "shift+/" => Some("?"),
        "shift+]" => Some("}"),
        "shift+[" => Some("{"),
        "shift+=" => Some("+"),
        "shift+-" => Some("_"),
        _ => None,
    }
}

fn title_case(value: &str) -> String {
    let mut chars = value.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_keymap_has_no_contextual_conflicts() {
        let overrides = Vec::new();
        for entry in shortcut_entries() {
            for binding in entry.keys {
                assert!(
                    binding_conflict(&overrides, entry, binding).is_none(),
                    "{} should not conflict on {}",
                    entry.description,
                    binding
                );
            }
        }
    }
}

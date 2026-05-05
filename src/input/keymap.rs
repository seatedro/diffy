#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShortcutEntry {
    pub key: &'static str,
    pub description: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShortcutGroup {
    pub title: &'static str,
    pub entries: &'static [ShortcutEntry],
}

const GROUPS: &[ShortcutGroup] = &[
    ShortcutGroup {
        title: "Navigation",
        entries: &[
            ShortcutEntry {
                key: "] / }",
                description: "Next hunk",
            },
            ShortcutEntry {
                key: "[ / {",
                description: "Previous hunk",
            },
            ShortcutEntry {
                key: "h / l",
                description: "Focus file list / diff",
            },
            ShortcutEntry {
                key: "j / k",
                description: "Move selection or scroll diff",
            },
            ShortcutEntry {
                key: "J / K",
                description: "Move diff row cursor",
            },
            ShortcutEntry {
                key: "n / N",
                description: "Next / previous file",
            },
            ShortcutEntry {
                key: "Tab",
                description: "Toggle sidebar / editor focus",
            },
            ShortcutEntry {
                key: "/",
                description: "Focus sidebar search",
            },
            ShortcutEntry {
                key: "F / C",
                description: "Files / commits sidebar tab",
            },
        ],
    },
    ShortcutGroup {
        title: "Scrolling",
        entries: &[
            ShortcutEntry {
                key: "d / u",
                description: "Scroll down / up half page",
            },
            ShortcutEntry {
                key: "Space / Shift+Space",
                description: "Page down / up",
            },
            ShortcutEntry {
                key: "g g / G",
                description: "Go to top / bottom",
            },
        ],
    },
    ShortcutGroup {
        title: "View",
        entries: &[
            ShortcutEntry {
                key: "1 / 2",
                description: "Unified / split diff view",
            },
            ShortcutEntry {
                key: "m",
                description: "Open compare menu",
            },
            ShortcutEntry {
                key: "m then 1-9",
                description: "Choose compare mode or preset",
            },
            ShortcutEntry {
                key: "r",
                description: "Refresh current view",
            },
            ShortcutEntry {
                key: "t",
                description: "Toggle file tree",
            },
            ShortcutEntry {
                key: "= / -",
                description: "Expand / collapse folders",
            },
            ShortcutEntry {
                key: "w",
                description: "Toggle line wrapping",
            },
            ShortcutEntry {
                key: "Cmd+B",
                description: "Toggle sidebar",
            },
        ],
    },
    ShortcutGroup {
        title: "Search",
        entries: &[
            ShortcutEntry {
                key: "Cmd+F",
                description: "Open search",
            },
            ShortcutEntry {
                key: "Enter / Shift+Enter",
                description: "Next / previous match",
            },
            ShortcutEntry {
                key: "Escape",
                description: "Close search",
            },
        ],
    },
    ShortcutGroup {
        title: "Settings",
        entries: &[
            ShortcutEntry {
                key: "1-5",
                description: "Switch settings section",
            },
            ShortcutEntry {
                key: "j / k",
                description: "Next / previous settings section",
            },
            ShortcutEntry {
                key: "t / b",
                description: "Toggle theme mode / browse themes",
            },
            ShortcutEntry {
                key: "w / c",
                description: "Toggle wrap / continuous scroll",
            },
            ShortcutEntry {
                key: "a / u",
                description: "Toggle auto-update / check updates",
            },
        ],
    },
    ShortcutGroup {
        title: "Working Tree",
        entries: &[
            ShortcutEntry {
                key: "s",
                description: "Stage file / hunk / selected lines",
            },
            ShortcutEntry {
                key: "u / S / U",
                description: "Unstage file / hunk / selected lines",
            },
            ShortcutEntry {
                key: "x",
                description: "Discard file / hunk / selected lines",
            },
            ShortcutEntry {
                key: "a / A",
                description: "Stage all / unstage all",
            },
            ShortcutEntry {
                key: "c",
                description: "Focus commit message",
            },
            ShortcutEntry {
                key: "Cmd+Enter",
                description: "Create commit from message",
            },
            ShortcutEntry {
                key: "v / V",
                description: "Select changed line / range",
            },
            ShortcutEntry {
                key: "R",
                description: "Comment on selected lines",
            },
        ],
    },
    ShortcutGroup {
        title: "Repository",
        entries: &[
            ShortcutEntry {
                key: "f",
                description: "Fetch remotes",
            },
            ShortcutEntry {
                key: "p",
                description: "Pull current branch",
            },
            ShortcutEntry {
                key: "P",
                description: "Publish options",
            },
            ShortcutEntry {
                key: "P then Enter / 1-9",
                description: "Run a publish action",
            },
            ShortcutEntry {
                key: "Cmd+P",
                description: "Command palette",
            },
            ShortcutEntry {
                key: "Cmd+P, jj",
                description: "Run jj operations and restore from operation log",
            },
            ShortcutEntry {
                key: "Enter / y",
                description: "Confirm guarded operation",
            },
            ShortcutEntry {
                key: "?",
                description: "Show this help",
            },
            ShortcutEntry {
                key: "Escape",
                description: "Close overlay / return from commit drilldown",
            },
        ],
    },
];

pub fn shortcut_groups() -> &'static [ShortcutGroup] {
    GROUPS
}

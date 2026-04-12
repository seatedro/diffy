use halogen::view;

use crate::ui::components;
use crate::ui::components::modal::Modal;
use crate::ui::design::{Sp, Sz};
use crate::ui::element::*;
use crate::ui::icons::lucide;
use crate::ui::state::AppState;
use crate::ui::style::Styled;

struct ShortcutEntry {
    key: &'static str,
    description: &'static str,
}

struct ShortcutGroup {
    title: &'static str,
    entries: &'static [ShortcutEntry],
}

const GROUPS: &[ShortcutGroup] = &[
    ShortcutGroup {
        title: "Navigation",
        entries: &[
            ShortcutEntry {
                key: "] / [",
                description: "Next / previous hunk",
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
        ],
    },
    ShortcutGroup {
        title: "Scrolling",
        entries: &[
            ShortcutEntry {
                key: "j / k",
                description: "Scroll down / up one line",
            },
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
        title: "General",
        entries: &[
            ShortcutEntry {
                key: "Cmd+P",
                description: "Command palette",
            },
            ShortcutEntry {
                key: "Cmd+= / Cmd+-",
                description: "Zoom in / out",
            },
            ShortcutEntry {
                key: "?",
                description: "Show this help",
            },
            ShortcutEntry {
                key: "Escape",
                description: "Close overlay",
            },
        ],
    },
];

fn build_keys_row(key: &str, theme: &crate::ui::theme::Theme) -> AnyElement {
    let tc = &theme.colors;
    let scale = theme.metrics.ui_scale();
    let parts: Vec<&str> = key.split(" / ").collect();

    view! { scale,
        <div class="shrink-0 flex-row items-center flex-wrap"
             min_w={Sz::CONTEXT_MENU_MIN_W}
             gap={Sp::XS}>
            for (i, part) in parts.iter().enumerate() {
                <fragment>
                    if i > 0 {
                        <text class="text-xs" color={tc.text_muted}>{"/"}</text>
                    }
                    for (j, sub) in part.split('+').enumerate() {
                        <fragment>
                            if j > 0 {
                                <text class="text-xs" color={tc.text_muted}>{"+"}</text>
                            }
                            {components::kbd(sub.trim(), theme)}
                        </fragment>
                    }
                </fragment>
            }
        </div>
    }
}

pub fn keyboard_shortcuts(
    _state: &AppState,
    theme: &crate::ui::theme::Theme,
    width: f32,
    height: f32,
) -> AnyElement {
    let tc = &theme.colors;
    let scale = theme.metrics.ui_scale();

    let body = view! { scale,
        <div class="flex-col" gap={Sp::XL}>
            for group in GROUPS {
                <div class="flex-col" gap={Sp::XS}>
                    <text class="text-sm font-semibold" color={tc.accent}>{group.title}</text>
                    for entry in group.entries {
                        <div class="flex-row items-center" gap={Sp::MD}>
                            {build_keys_row(entry.key, theme)}
                            <text class="text-sm" color={tc.text_muted}>{entry.description}</text>
                        </div>
                    }
                </div>
            }
        </div>
    };

    view! { scale,
        <Modal title={"Keyboard Shortcuts"}
               subtitle={"Press ? to dismiss"}
               icon={lucide::COMMAND}
               max_width={Sz::MODAL_LG * scale}
               window_width={width}
               window_height={height}>
            <Body>{body}</Body>
        </Modal>
    }
}

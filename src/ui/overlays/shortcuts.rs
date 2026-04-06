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

pub fn keyboard_shortcuts(
    _state: &AppState,
    theme: &crate::ui::theme::Theme,
    width: f32,
    height: f32,
) -> AnyElement {
    let tc = &theme.colors;
    let scale = theme.metrics.ui_scale();

    let mut body = div().flex_col().gap((Sp::XL * scale).round());

    for group in GROUPS {
        let mut section = div().flex_col().gap((Sp::XS * scale).round());

        section = section.child(view! {
            <text class="text-sm font-semibold" color={tc.accent}>{group.title}</text>
        });

        for entry in group.entries {
            let mut keys_row = div()
                .flex_shrink_0()
                .min_w(Sz::CONTEXT_MENU_MIN_W)
                .flex_row()
                .items_center()
                .flex_wrap()
                .gap((Sp::XS * scale).round());

            let parts: Vec<&str> = entry.key.split(" / ").collect();
            for (i, part) in parts.iter().enumerate() {
                if i > 0 {
                    keys_row = keys_row.child(text("/").text_xs().color(tc.text_muted));
                }
                let subparts: Vec<&str> = part.split('+').collect();
                for (j, sub) in subparts.iter().enumerate() {
                    if j > 0 {
                        keys_row = keys_row.child(text("+").text_xs().color(tc.text_muted));
                    }
                    keys_row = keys_row.child(components::kbd(sub.trim(), theme));
                }
            }

            section = section.child(view! { scale,
                <div class="flex-row items-center" gap={Sp::MD}>
                    {keys_row}
                    <text class="text-sm" color={tc.text_muted}>{entry.description}</text>
                </div>
            });
        }

        body = body.child(section);
    }

    Modal::new(
        "Keyboard Shortcuts",
        "Press ? to dismiss",
        lucide::COMMAND,
        Sz::MODAL_LG * scale,
        width,
        height,
    )
    .body_child(body)
    .into_any()
}

use quark::view;
use quark_components::{kbd, kbd_icon};
use quark_ui::element::*;
use quark_ui::icons::lucide;
use quark_ui::style::Styled;
use quark_ui::theme::Theme;

use super::keymap;
use crate::app::icons;
use crate::app::tokens::{self, Sp};
use crate::core::keymap::{KeymapOverride, ShortcutCommand};

pub fn command(
    saved: &[KeymapOverride],
    command: ShortcutCommand,
    theme: &Theme,
) -> Option<AnyElement> {
    keymap::first_binding(saved, command).map(|binding| caps(&binding, theme))
}

pub fn caps(binding: &str, theme: &Theme) -> AnyElement {
    let scale = tokens::ui_scale(theme);
    let mut keys: Vec<AnyElement> = Vec::new();
    for stroke in binding.split_whitespace() {
        if let Some(symbol) = shifted(stroke) {
            keys.push(kbd(symbol, theme));
            continue;
        }
        let mut parts: Vec<&str> = stroke.split('+').collect();
        let key = parts.pop().unwrap_or_default();
        for modifier in ["ctrl", "alt", "shift", "mod", "cmd"] {
            if parts.contains(&modifier) && !(modifier == "cmd" && parts.contains(&"mod")) {
                keys.push(modifier_cap(modifier, theme));
            }
        }
        keys.push(key_cap(key, theme));
    }
    view! { scale,
        <div class="flex-row items-center shrink-0" gap={Sp::XXS}>{...keys}</div>
    }
}

fn modifier_cap(modifier: &str, theme: &Theme) -> AnyElement {
    if cfg!(target_os = "macos") {
        let icon = match modifier {
            "ctrl" => lucide::CHEVRON_UP,
            "alt" => icons::OPTION,
            "shift" => icons::SHIFT,
            _ => lucide::COMMAND,
        };
        return kbd_icon(icon, theme);
    }
    kbd(
        match modifier {
            "alt" => "Alt",
            "shift" => "Shift",
            _ => "Ctrl",
        },
        theme,
    )
}

fn key_cap(key: &str, theme: &Theme) -> AnyElement {
    let icon = match key {
        "enter" => Some(icons::CORNER_DOWN_LEFT),
        "tab" => Some(icons::ARROW_RIGHT_TO_LINE),
        "arrowup" => Some(lucide::ARROW_UP),
        "arrowdown" => Some(lucide::ARROW_DOWN),
        "arrowright" => Some(lucide::ARROW_RIGHT),
        _ => None,
    };
    if let Some(icon) = icon {
        return kbd_icon(icon, theme);
    }
    let label = match key {
        "escape" => "esc".to_owned(),
        "space" => "space".to_owned(),
        "/" => "/".to_owned(),
        key if key.chars().count() == 1 => key.to_uppercase(),
        key => key.to_owned(),
    };
    kbd(label, theme)
}

fn shifted(stroke: &str) -> Option<&'static str> {
    Some(match stroke {
        "shift+/" => "?",
        "shift+]" => "}",
        "shift+[" => "{",
        "shift+=" => "+",
        "shift+-" => "_",
        _ => return None,
    })
}

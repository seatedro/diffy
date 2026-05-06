use halogen::view;

use crate::input::{format_binding, shortcut_groups};
use crate::ui::components;
use crate::ui::components::modal::Modal;
use crate::ui::design::{Sp, Sz};
use crate::ui::element::*;
use crate::ui::icons::lucide;
use crate::ui::state::AppState;
use crate::ui::style::Styled;

fn build_keys_row(keys: &[&str], theme: &crate::ui::theme::Theme) -> AnyElement {
    let tc = &theme.colors;
    let scale = theme.metrics.ui_scale();
    let parts: Vec<String> = keys.iter().map(|key| format_binding(key)).collect();

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
            for group in shortcut_groups() {
                <div class="flex-col" gap={Sp::XS}>
                    <text class="text-sm font-semibold" color={tc.accent}>{group.title}</text>
                    for entry in group.entries {
                        <div class="flex-row items-center" gap={Sp::MD}>
                            {build_keys_row(entry.keys, theme)}
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

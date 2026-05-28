use halogen::view;

use crate::actions::{Action, ContextMenuEntry};
use crate::render::Rect;
use crate::ui::design::{Shadow, Sp, Sz};
use crate::ui::element::{AnyElement, IntoAnyElement, div, svg_icon, text};
use crate::ui::style::Styled;
use crate::ui::theme::Theme;

pub fn context_menu_layer(
    entries: Vec<ContextMenuEntry>,
    x: f32,
    y: f32,
    theme: &Theme,
) -> AnyElement {
    let tc = &theme.colors;
    let m = &theme.metrics;
    let scale = m.ui_scale();

    view! {
        <div class="absolute flex-col" left={x} top={y} z_index={250}
             min_w={Sz::CONTEXT_MENU_MIN_W * scale}
             py={m.spacing_xs}
             bg={tc.elevated_surface} border={tc.border}
             rounded={m.panel_radius} shadow_preset={Shadow::CONTEXT_MENU}
             on_click={Action::Noop}>
            for entry in entries {
                match entry {
                    ContextMenuEntry::Item { label, icon, action, shortcut, destructive, disabled } => {
                        let accessibility_label = label.clone();
                        let accessibility_id = format!("context-menu:{:?}:{accessibility_label}", action);
                        let fg = if disabled {
                            tc.text_muted
                        } else if destructive {
                            tc.status_error
                        } else {
                            tc.text
                        };
                        let icon_color = if disabled {
                            tc.text_muted
                        } else if destructive {
                            tc.status_error
                        } else {
                            tc.icon
                        };
                        let icon_size = m.ui_small_font_size;
                        view! {
                            <div class="flex-row items-center"
                                 gap={m.spacing_sm} px={m.spacing_md}
                                 py={m.spacing_xs + Sp::XXS * scale}
                                 accessibility_role={accesskit::Role::MenuItem}
                                 accessibility_id={accessibility_id}
                                 accessibility_label={accessibility_label}
                                 accessibility_disabled={disabled}
                                 @when {!disabled} { on_click={action} hover_bg={tc.sidebar_row_hover} }>
                                if let Some(svg) = icon {
                                    <icon svg={svg} size={icon_size} color={icon_color} />
                                } else {
                                    <div class="shrink-0" w={icon_size} h={icon_size} />
                                }
                                <div class="flex-1">
                                    <text class="text-sm" color={fg}>{label}</text>
                                </div>
                                if let Some(key) = shortcut {
                                    <text class="text-xs" color={tc.text_muted}>{key}</text>
                                }
                            </div>
                        }
                    }
                    ContextMenuEntry::Separator => {
                        view! {
                            <div py={m.spacing_xs} px={m.spacing_sm}>
                                <div class="w-full" h={Sz::SEPARATOR_W} bg={tc.border_variant} />
                            </div>
                        }
                    }
                }
            }
        </div>
    }
}

#[derive(Debug, Clone)]
pub struct ContextMenuState {
    pub entries: Vec<ContextMenuEntry>,
    pub x: f32,
    pub y: f32,
    pub visible: bool,
    pub bounds: Option<Rect>,
}

impl Default for ContextMenuState {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
            x: 0.0,
            y: 0.0,
            visible: false,
            bounds: None,
        }
    }
}

impl ContextMenuState {
    pub fn open(&mut self, entries: Vec<ContextMenuEntry>, x: f32, y: f32) {
        self.entries = entries;
        self.x = x;
        self.y = y;
        self.visible = true;
        self.bounds = None;
    }

    pub fn close(&mut self) {
        self.visible = false;
        self.entries.clear();
        self.bounds = None;
    }

    pub fn render(&mut self, theme: &Theme) -> Option<AnyElement> {
        if self.visible && !self.entries.is_empty() {
            self.bounds = Some(self.estimated_bounds(theme));
            let result = context_menu_layer(self.entries.clone(), self.x, self.y, theme);
            Some(result)
        } else {
            self.bounds = None;
            None
        }
    }

    pub fn contains(&self, x: f32, y: f32) -> bool {
        self.bounds.is_some_and(|bounds| bounds.contains(x, y))
    }

    fn estimated_bounds(&self, theme: &Theme) -> Rect {
        let m = &theme.metrics;
        let scale = m.ui_scale();
        let item_h = (m.ui_small_font_size * 1.35 + (m.spacing_xs + Sp::XXS * scale) * 2.0)
            .ceil()
            .max(24.0 * scale);
        let separator_h = (m.spacing_xs * 2.0 + Sz::SEPARATOR_W).ceil();
        let entries_h = self.entries.iter().fold(0.0, |height, entry| {
            height
                + match entry {
                    ContextMenuEntry::Item { .. } => item_h,
                    ContextMenuEntry::Separator => separator_h,
                }
        });
        Rect {
            x: self.x,
            y: self.y,
            width: Sz::CONTEXT_MENU_MIN_W * scale,
            height: m.spacing_xs * 2.0 + entries_h,
        }
    }
}

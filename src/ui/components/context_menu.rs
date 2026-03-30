use crate::ui::actions::Action;
use crate::ui::element::{div, svg_icon, text, Div};
use crate::ui::style::Styled;
use crate::ui::theme::{Color, Theme};

pub enum ContextMenuEntry {
    Item {
        label: String,
        icon: Option<&'static str>,
        action: Action,
        shortcut: Option<String>,
        destructive: bool,
        disabled: bool,
    },
    Separator,
}

impl ContextMenuEntry {
    pub fn item(label: impl Into<String>, action: Action) -> Self {
        Self::Item {
            label: label.into(),
            icon: None,
            action,
            shortcut: None,
            destructive: false,
            disabled: false,
        }
    }

    pub fn icon(mut self, svg: &'static str) -> Self {
        if let Self::Item { icon, .. } = &mut self {
            *icon = Some(svg);
        }
        self
    }

    pub fn shortcut(mut self, s: impl Into<String>) -> Self {
        if let Self::Item { shortcut, .. } = &mut self {
            *shortcut = Some(s.into());
        }
        self
    }

    pub fn destructive(mut self) -> Self {
        if let Self::Item { destructive, .. } = &mut self {
            *destructive = true;
        }
        self
    }

    pub fn disabled(mut self) -> Self {
        if let Self::Item { disabled, .. } = &mut self {
            *disabled = true;
        }
        self
    }

    pub fn separator() -> Self {
        Self::Separator
    }
}

pub fn context_menu_layer(entries: Vec<ContextMenuEntry>, x: f32, y: f32, theme: &Theme) -> Div {
    let tc = &theme.colors;
    let m = &theme.metrics;

    let mut menu = div()
        .absolute()
        .left(x)
        .top(y)
        .z_index(250)
        .flex_col()
        .min_w(180.0)
        .py(m.spacing_xs)
        .bg(tc.elevated_surface)
        .border(tc.border)
        .rounded(m.panel_radius)
        .shadow(16.0, 8.0, Color::rgba(0, 0, 0, 50))
        .shadow(4.0, 2.0, Color::rgba(0, 0, 0, 30))
        .shadow(1.0, 0.0, Color::rgba(0, 0, 0, 15));

    for entry in entries {
        match entry {
            ContextMenuEntry::Item {
                label,
                icon,
                action,
                shortcut,
                destructive,
                disabled,
            } => {
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

                let mut row = div()
                    .flex_row()
                    .items_center()
                    .gap(m.spacing_sm)
                    .px(m.spacing_md)
                    .py(m.spacing_xs + 2.0);

                if !disabled {
                    row = row.on_click(action).hover_bg(tc.ghost_element_hover);
                }

                if let Some(svg) = icon {
                    row = row.child(svg_icon(svg, icon_size).color(icon_color));
                } else {
                    row = row.child(div().w(icon_size).h(icon_size).flex_shrink_0());
                }

                row = row.child(
                    div()
                        .flex_1()
                        .child(text(label).text_sm().color(fg)),
                );

                if let Some(key) = shortcut {
                    row = row.child(text(key).text_xs().color(tc.text_muted));
                }

                menu = menu.child(row);
            }
            ContextMenuEntry::Separator => {
                menu = menu.child(
                    div()
                        .py(m.spacing_xs)
                        .px(m.spacing_sm)
                        .child(div().w_full().h(1.0).bg(tc.border_variant)),
                );
            }
        }
    }

    menu
}

pub struct ContextMenuState {
    pub entries: Vec<ContextMenuEntry>,
    pub x: f32,
    pub y: f32,
    pub visible: bool,
}

impl Default for ContextMenuState {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
            x: 0.0,
            y: 0.0,
            visible: false,
        }
    }
}

impl ContextMenuState {
    pub fn open(&mut self, entries: Vec<ContextMenuEntry>, x: f32, y: f32) {
        self.entries = entries;
        self.x = x;
        self.y = y;
        self.visible = true;
    }

    pub fn close(&mut self) {
        self.visible = false;
        self.entries.clear();
    }

    pub fn render(&mut self, theme: &Theme) -> Option<Div> {
        if self.visible && !self.entries.is_empty() {
            let entries = std::mem::take(&mut self.entries);
            let result = context_menu_layer(entries, self.x, self.y, theme);
            Some(result)
        } else {
            None
        }
    }
}

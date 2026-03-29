use crate::render::{FontKind, Rect, RoundedRectPrimitive, TextPrimitive};
use crate::ui::actions::Action;
use crate::ui::design::Sp;
use crate::ui::shell::{CursorHint, HitRegion, UiFrame};
use crate::ui::theme::{Color, Theme};

pub struct ListItem<'a> {
    title: &'a str,
    detail: Option<String>,
    selected: bool,
    hover_progress: f32,
    on_click: Option<Action>,
    hover_file_index: Option<usize>,
}

impl<'a> ListItem<'a> {
    pub fn new(title: &'a str) -> Self {
        Self {
            title,
            detail: None,
            selected: false,
            hover_progress: 0.0,
            on_click: None,
            hover_file_index: None,
        }
    }

    pub fn detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    pub fn hovered(self, _hovered: bool) -> Self {
        self
    }

    pub fn hover_progress(mut self, progress: f32) -> Self {
        self.hover_progress = progress;
        self
    }

    pub fn on_click(mut self, action: Action) -> Self {
        self.on_click = Some(action);
        self
    }

    pub fn hover_file_index(mut self, index: usize) -> Self {
        self.hover_file_index = Some(index);
        self
    }

    pub fn paint(self, frame: &mut UiFrame, rect: Rect, theme: &Theme) {
        if self.selected {
            frame.scene.rounded_rect(RoundedRectPrimitive::uniform(
                rect,
                theme.metrics.control_radius,
                theme.colors.sidebar_row_selected,
            ));
        } else if self.hover_progress > 0.001 {
            let transparent = Color::rgba(0, 0, 0, 0);
            let hover_color = theme.colors.sidebar_row_hover;
            frame.scene.rounded_rect(RoundedRectPrimitive::uniform(
                rect,
                theme.metrics.control_radius,
                transparent.lerp(hover_color, self.hover_progress),
            ));
        }

        let title_lh = theme.metrics.ui_font_size * 1.5;
        let detail_lh = theme.metrics.ui_small_font_size * 1.5;
        let pad_x = Sp::MD;
        let pad_y = Sp::XS;
        frame.scene.text(TextPrimitive {
            rect: Rect {
                x: rect.x + pad_x,
                y: rect.y + pad_y,
                width: rect.width - pad_x * 2.0,
                height: title_lh,
            },
            text: self.title.to_owned(),
            color: theme.colors.text,
            font_size: theme.metrics.ui_font_size,
            font_kind: FontKind::Ui,
        });

        if let Some(detail) = &self.detail {
            frame.scene.text(TextPrimitive {
                rect: Rect {
                    x: rect.x + pad_x,
                    y: rect.y + pad_y + title_lh,
                    width: rect.width - pad_x * 2.0,
                    height: detail_lh,
                },
                text: detail.clone(),
                color: theme.colors.text_muted,
                font_size: theme.metrics.ui_small_font_size,
                font_kind: FontKind::Ui,
            });
        }

        if let Some(action) = self.on_click {
            frame.hits.push(HitRegion {
                rect,
                action,
                hover_file_index: self.hover_file_index,
                hover_toast_index: None,
                cursor: CursorHint::Pointer,
            });
        }
    }
}

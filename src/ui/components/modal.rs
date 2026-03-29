use crate::render::{Rect, RectPrimitive};
use crate::ui::actions::Action;
use crate::ui::design::{Elevation, TextStyle};
use crate::ui::shell::{CursorHint, HitRegion, UiFrame};
use crate::ui::theme::Theme;

use super::Label;

pub struct Modal;

impl Modal {
    pub fn backdrop(frame: &mut UiFrame, theme: &Theme, width: f32, height: f32) {
        let rect = Rect {
            x: 0.0,
            y: 0.0,
            width,
            height,
        };
        frame.scene.rect(RectPrimitive {
            rect,
            color: theme.colors.overlay_scrim,
        });
        frame.hits.push(HitRegion {
            rect,
            action: Action::CloseOverlay,
            hover_file_index: None,
            hover_toast_index: None,
            cursor: CursorHint::Default,
        });
    }

    pub fn panel(frame: &mut UiFrame, rect: Rect, theme: &Theme) {
        Elevation::Modal.paint_default(frame, rect, theme);
    }

    pub fn header(
        frame: &mut UiFrame,
        rect: Rect,
        title: &str,
        subtitle: &str,
        theme: &Theme,
    ) {
        Label::new(title)
            .style(TextStyle::Heading)
            .paint(
                frame,
                Rect {
                    x: rect.x + 24.0,
                    y: rect.y + 20.0,
                    width: rect.width - 48.0,
                    height: 20.0,
                },
                theme,
            );
        if !subtitle.is_empty() {
            Label::new(subtitle)
                .style(TextStyle::BodySmall)
                .paint(
                    frame,
                    Rect {
                        x: rect.x + 24.0,
                        y: rect.y + 44.0,
                        width: rect.width - 48.0,
                        height: 16.0,
                    },
                    theme,
                );
        }
    }
}

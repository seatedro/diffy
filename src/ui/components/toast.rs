use crate::render::Rect;
use crate::ui::actions::Action;
use crate::ui::design::{Elevation, Sp, TextStyle};
use crate::ui::shell::{CursorHint, HitRegion, UiFrame};
use crate::ui::state::ToastKind;
use crate::ui::theme::Theme;

use super::{Button, Label};

pub struct Toast<'a> {
    message: &'a str,
    kind: ToastKind,
    index: usize,
}

impl<'a> Toast<'a> {
    pub fn new(message: &'a str, kind: ToastKind, index: usize) -> Self {
        Self {
            message,
            kind,
            index,
        }
    }

    pub fn paint(self, frame: &mut UiFrame, rect: Rect, theme: &Theme) {
        let fill = match self.kind {
            ToastKind::Info => theme.colors.elevated_surface,
            ToastKind::Error => theme.colors.modal_surface,
        };
        Elevation::Raised.paint(frame, rect, fill, theme.colors.border, theme);

        Label::new(self.message)
            .style(TextStyle::BodySmall)
            .paint(
                frame,
                Rect {
                    x: rect.x + Sp::XL,
                    y: rect.y + Sp::LG,
                    width: rect.width - 68.0,
                    height: 16.0,
                },
                theme,
            );

        let dismiss = Rect {
            x: rect.right() - 42.0,
            y: rect.y + Sp::LG,
            width: 28.0,
            height: 28.0,
        };
        Button::new("x", Action::DismissToast(self.index)).paint(frame, dismiss, theme);

        frame.hits.push(HitRegion {
            rect,
            action: Action::DismissToast(self.index),
            hover_file_index: None,
            hover_toast_index: Some(self.index),
            cursor: CursorHint::Pointer,
        });
    }
}

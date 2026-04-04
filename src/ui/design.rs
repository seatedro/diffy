use crate::render::{BorderPrimitive, FontKind, Rect, RoundedRectPrimitive, ShadowPrimitive};
use crate::ui::shell::UiFrame;
use crate::ui::theme::{Color, Theme};

pub struct Sp;

impl Sp {
    pub const NONE: f32 = 0.0;
    pub const XXS: f32 = 2.0;
    pub const XS: f32 = 4.0;
    pub const SM: f32 = 8.0;
    pub const MD: f32 = 12.0;
    pub const LG: f32 = 16.0;
    pub const XL: f32 = 20.0;
    pub const XXL: f32 = 28.0;
    pub const XXXL: f32 = 40.0;
    pub const XXXXL: f32 = 56.0;
}

pub struct Sz;

impl Sz {
    pub const ROW: f32 = 36.0;
    pub const INPUT: f32 = 44.0;
    pub const INPUT_LABELED: f32 = 64.0;
    pub const SEARCH_INPUT: f32 = 28.0;
    pub const TOAST: f32 = 52.0;
    pub const SEPARATOR_W: f32 = 1.0;
    pub const SEPARATOR_H: f32 = 20.0;
    pub const MODE_TOGGLE: f32 = 22.0;
    pub const TOAST_MAX_W: f32 = 360.0;
    pub const TOAST_MIN_W: f32 = 220.0;
    pub const TOAST_STRIPE_W: f32 = 3.0;
    pub const TOAST_MARGIN: f32 = 32.0;
    pub const MODAL_MARGIN: f32 = 48.0;
    pub const MODAL_TOP_OFFSET: f32 = 80.0;
    pub const MODAL_SM: f32 = 480.0;
    pub const MODAL_MD: f32 = 560.0;
    pub const MODAL_LG: f32 = 640.0;
    pub const MODAL_XL: f32 = 680.0;
    pub const SIDEBAR_LIST_OFFSET: f32 = 40.0;
    pub const CARD_SM: f32 = 440.0;
    pub const CARD_MD: f32 = 520.0;
    pub const CARD_AUTH: f32 = 580.0;
}

pub struct Ico;

impl Ico {
    pub const XS: f32 = 12.0;
    pub const SM: f32 = 14.0;
    pub const MD: f32 = 15.0;
    pub const LG: f32 = 18.0;
    pub const XL: f32 = 20.0;
    pub const XXL: f32 = 24.0;
    pub const HERO: f32 = 32.0;
    pub const SIDEBAR_MODE: f32 = 13.0;
    pub const BUTTON_COMPACT: f32 = 14.0;
    pub const BUTTON_DEFAULT: f32 = 15.0;
}

pub struct Rad;

impl Rad {
    pub const SM: f32 = 4.0;
    pub const MD: f32 = 5.0;
    pub const LG: f32 = 6.0;
    pub const XL: f32 = 12.0;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextStyle {
    HeadingLg,
    Heading,
    Body,
    BodySmall,
    Caption,
    Mono,
    MonoSmall,
}

impl TextStyle {
    pub fn font_size(self, theme: &Theme) -> f32 {
        match self {
            Self::HeadingLg => theme.metrics.heading_font_size + 3.0,
            Self::Heading => theme.metrics.heading_font_size,
            Self::Body => theme.metrics.ui_font_size,
            Self::BodySmall => theme.metrics.ui_small_font_size,
            Self::Caption => theme.metrics.ui_small_font_size - 1.0,
            Self::Mono => theme.metrics.mono_font_size,
            Self::MonoSmall => theme.metrics.mono_font_size - 1.0,
        }
    }

    pub fn font_kind(self) -> FontKind {
        match self {
            Self::Mono | Self::MonoSmall => FontKind::Mono,
            _ => FontKind::Ui,
        }
    }

    pub fn color(self, theme: &Theme) -> Color {
        match self {
            Self::HeadingLg | Self::Heading => theme.colors.text_strong,
            Self::Body | Self::Mono => theme.colors.text,
            Self::BodySmall | Self::Caption | Self::MonoSmall => theme.colors.text_muted,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Elevation {
    Surface,
    Raised,
    Popover,
    Modal,
}

impl Elevation {
    pub fn radius(self, theme: &Theme) -> f32 {
        match self {
            Self::Surface => theme.metrics.panel_radius,
            Self::Raised => theme.metrics.panel_radius + 2.0,
            Self::Popover => theme.metrics.panel_radius,
            Self::Modal => theme.metrics.modal_radius,
        }
    }

    pub fn default_fill(self, theme: &Theme) -> Color {
        match self {
            Self::Surface => theme.colors.surface,
            Self::Raised => theme.colors.elevated_surface,
            Self::Popover => theme.colors.elevated_surface,
            Self::Modal => theme.colors.modal_surface,
        }
    }

    pub fn default_border(self, theme: &Theme) -> Color {
        match self {
            Self::Surface => theme.colors.border_variant,
            Self::Raised | Self::Popover => theme.colors.border,
            Self::Modal => theme.colors.border,
        }
    }

    pub fn paint(self, frame: &mut UiFrame, rect: Rect, fill: Color, border: Color, theme: &Theme) {
        let radius = self.radius(theme);
        match self {
            Self::Surface => {}
            Self::Raised => {
                frame.scene.shadow(ShadowPrimitive {
                    rect,
                    blur_radius: 3.0,
                    corner_radius: radius,
                    offset: [0.0, 2.0],
                    color: Color::rgba(0, 0, 0, 30),
                });
                frame.scene.shadow(ShadowPrimitive {
                    rect,
                    blur_radius: 18.0,
                    corner_radius: radius,
                    offset: [0.0, 4.0],
                    color: Color::rgba(0, 0, 0, 50),
                });
            }
            Self::Popover => {
                frame.scene.shadow(ShadowPrimitive {
                    rect,
                    blur_radius: 3.0,
                    corner_radius: radius,
                    offset: [0.0, 2.0],
                    color: Color::rgba(0, 0, 0, 25),
                });
                frame.scene.shadow(ShadowPrimitive {
                    rect,
                    blur_radius: 8.0,
                    corner_radius: radius,
                    offset: [0.0, 4.0],
                    color: Color::rgba(0, 0, 0, 35),
                });
                frame.scene.shadow(ShadowPrimitive {
                    rect,
                    blur_radius: 16.0,
                    corner_radius: radius,
                    offset: [0.0, 6.0],
                    color: Color::rgba(0, 0, 0, 25),
                });
            }
            Self::Modal => {
                frame.scene.shadow(ShadowPrimitive {
                    rect,
                    blur_radius: 3.0,
                    corner_radius: radius,
                    offset: [0.0, 2.0],
                    color: Color::rgba(0, 0, 0, 30),
                });
                frame.scene.shadow(ShadowPrimitive {
                    rect,
                    blur_radius: 8.0,
                    corner_radius: radius,
                    offset: [0.0, 4.0],
                    color: Color::rgba(0, 0, 0, 20),
                });
                frame.scene.shadow(ShadowPrimitive {
                    rect,
                    blur_radius: 24.0,
                    corner_radius: radius,
                    offset: [0.0, 8.0],
                    color: Color::rgba(0, 0, 0, 50),
                });
                frame.scene.shadow(ShadowPrimitive {
                    rect,
                    blur_radius: 1.0,
                    corner_radius: radius,
                    offset: [0.0, 1.0],
                    color: Color::rgba(0, 0, 0, 15),
                });
            }
        }
        frame
            .scene
            .rounded_rect(RoundedRectPrimitive::uniform(rect, radius, fill));
        frame
            .scene
            .border(BorderPrimitive::uniform(rect, 1.0, radius, border));
    }

    pub fn paint_default(self, frame: &mut UiFrame, rect: Rect, theme: &Theme) {
        self.paint(
            frame,
            rect,
            self.default_fill(theme),
            self.default_border(theme),
            theme,
        );
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum InteractionState {
    #[default]
    Default,
    Hovered,
    Active,
    Selected,
    Focused,
    Disabled,
}

impl InteractionState {
    pub fn element_fill(self, theme: &Theme) -> Color {
        match self {
            Self::Default => theme.colors.element_background,
            Self::Hovered => theme.colors.element_hover,
            Self::Active => theme.colors.element_active,
            Self::Selected => theme.colors.element_selected,
            Self::Focused => theme.colors.element_hover,
            Self::Disabled => theme.colors.element_background,
        }
    }

    pub fn ghost_fill(self, theme: &Theme) -> Color {
        match self {
            Self::Default => Color::rgba(0, 0, 0, 0),
            Self::Hovered => theme.colors.ghost_element_hover,
            Self::Active => theme.colors.ghost_element_active,
            Self::Selected => theme.colors.ghost_element_selected,
            Self::Focused => theme.colors.ghost_element_hover,
            Self::Disabled => Color::rgba(0, 0, 0, 0),
        }
    }

    pub fn border(self, theme: &Theme) -> Color {
        match self {
            Self::Focused => theme.colors.focus_border,
            _ => theme.colors.border,
        }
    }

    pub fn text(self, theme: &Theme) -> Color {
        match self {
            Self::Disabled => theme.colors.text_muted,
            _ => theme.colors.text,
        }
    }
}

pub use quark_ui::design::{
    Alpha, Bp, Elevation, Ico, Rad, Shadow, ShadowLayer, Sp, Sz, TextStyle,
};
use quark_ui::theme::Theme;

pub struct Shell;

impl Shell {
    pub const SIDEBAR_MIN_FACTOR: f32 = 0.64;
    pub const TRAFFIC_LIGHT_MARGIN: f32 = 16.0;
    pub const TRAFFIC_LIGHTS_WIDTH: f32 = 54.0;
    pub const DEFAULT_WINDOW: (f64, f64) = (1320.0, 840.0);
    pub const MIN_WINDOW: (f64, f64) = (640.0, 480.0);
}

pub fn ui_scale(theme: &Theme) -> f32 {
    theme.metrics.ui_scale()
}

pub fn scaled(value: f32, scale: f32) -> f32 {
    (value * scale).round()
}

pub fn sidebar_default(theme: &Theme) -> f32 {
    theme.metrics.sidebar_width
}

pub fn sidebar_min(theme: &Theme) -> f32 {
    (theme.metrics.sidebar_width * Shell::SIDEBAR_MIN_FACTOR).round()
}

pub fn main_surface_min(theme: &Theme) -> f32 {
    scaled(Sz::MAIN_SURFACE_MIN_W, ui_scale(theme))
}

pub fn sidebar_fits(theme: &Theme, width: f32) -> bool {
    width >= Bp::COMPACT * ui_scale(theme)
}

pub fn title_leading_inset(theme: &Theme) -> f32 {
    let gap = scaled(Sp::MD, ui_scale(theme));
    if cfg!(target_os = "macos") {
        Shell::TRAFFIC_LIGHT_MARGIN + Shell::TRAFFIC_LIGHTS_WIDTH + gap
    } else {
        gap
    }
}

pub struct Layout;

impl Layout {
    pub const REF_PICKER_W: f32 = 470.0;
    pub const HOVER_CARD_W: f32 = 330.0;
    pub const POPOVER_GAP: f32 = 4.0;
    pub const COMPARE_CHIP_H: f32 = 26.0;
    pub const COMPARE_BAR_H: f32 = 32.0;
    pub const COMPARE_ARROW_MARGIN: f32 = 6.0;
    pub const SETTINGS_SHEET_W: f32 = 980.0;
    pub const SETTINGS_SHEET_H: f32 = 600.0;
    pub const SETTINGS_SHEET_NAV_W: f32 = 210.0;
    pub const SETTINGS_PREVIEW_W: f32 = 330.0;
    pub const HISTORY_ROW_H: f32 = 46.0;
    pub const AVATAR_SM: f32 = 20.0;
    pub const AVATAR_MD: f32 = 28.0;
    pub const STATUS_BADGE: f32 = 15.0;
    pub const PROGRESS_H: f32 = 4.0;
}

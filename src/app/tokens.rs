pub use quark_ui::design::{
    Alpha, Bp, Elevation, Ico, Rad, Shadow, ShadowLayer, Sp, Sz, TextStyle,
};
use quark_ui::theme::Theme;

pub struct Shell;

impl Shell {
    pub const SIDEBAR_MIN_FACTOR: f32 = 0.64;
    pub const TRAFFIC_LIGHT_MARGIN: f32 = 16.0;
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
    if cfg!(target_os = "macos") {
        Sz::CHROME_TRAFFIC_INSET
    } else {
        scaled(Sp::MD, ui_scale(theme))
    }
}

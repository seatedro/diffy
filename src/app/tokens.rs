use quark_ui::design::Ico as QuarkIco;
pub use quark_ui::design::{Alpha, Bp, Elevation, Rad, Shadow, ShadowLayer, Sp, Sz, TextStyle};
use quark_ui::theme::{Color, Theme, ThemeMetrics};

pub const METRICS: ThemeMetrics = ThemeMetrics {
    title_bar_height: 42.0,
    status_bar_height: 26.0,
    sidebar_width: 292.0,
    panel_radius: 10.0,
    control_radius: 7.0,
    modal_radius: 12.0,
    spacing_xs: 4.0,
    spacing_sm: 6.0,
    spacing_md: 12.0,
    spacing_lg: 16.0,
    ui_font_size: 13.0,
    ui_small_font_size: 12.0,
    heading_font_size: 15.0,
    mono_font_size: 12.5,
    ui_row_height: 30.0,
    code_row_height: 22.0,
};

const QUARK_ANCHOR_FONT: f32 = 16.0;

pub const fn quark_units(points: f32) -> f32 {
    points * QUARK_ANCHOR_FONT / METRICS.ui_font_size
}

pub struct Ico;

impl Ico {
    pub const XS: f32 = quark_units(QuarkIco::XS);
    pub const SM: f32 = quark_units(QuarkIco::SM);
    pub const MD: f32 = quark_units(QuarkIco::MD);
    pub const LG: f32 = quark_units(QuarkIco::LG);
    pub const XL: f32 = quark_units(QuarkIco::XL);
    pub const XXL: f32 = quark_units(QuarkIco::XXL);
    pub const HERO: f32 = quark_units(QuarkIco::HERO);
    pub const BUTTON_COMPACT: f32 = quark_units(QuarkIco::BUTTON_COMPACT);
    pub const BUTTON_DEFAULT: f32 = quark_units(QuarkIco::BUTTON_DEFAULT);
}

pub struct Shell;

impl Shell {
    pub const SIDEBAR_MIN_FACTOR: f32 = 0.64;
    pub const TRAFFIC_LIGHT_MARGIN: f32 = 16.0;
    pub const TRAFFIC_LIGHTS_WIDTH: f32 = 54.0;
    pub const DEFAULT_WINDOW: (f64, f64) = (1320.0, 840.0);
    pub const MIN_WINDOW: (f64, f64) = (640.0, 480.0);
}

pub fn ui_scale(theme: &Theme) -> f32 {
    theme.metrics.ui_font_size / METRICS.ui_font_size
}

pub struct Tint;

impl Tint {
    pub const SELECTED: f32 = 0.2;
    pub const SELECTED_IDLE: f32 = 0.08;
    pub const BADGE: f32 = 0.16;
}

pub fn selected_fill(base: Color, focused: bool, theme: &Theme) -> Color {
    if focused {
        base.lerp(theme.colors.accent, Tint::SELECTED)
    } else {
        base.lerp(theme.colors.text, Tint::SELECTED_IDLE)
    }
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
    pub const REF_PICKER_W: f32 = 540.0;
    pub const REPO_PICKER_W: f32 = 480.0;
    pub const AGE_W: f32 = 40.0;
    pub const BRANCH_MAX_W: f32 = 150.0;
    pub const HOVER_CARD_W: f32 = 330.0;
    pub const POPOVER_GAP: f32 = 4.0;
    pub const CONTROL_H: f32 = 26.0;
    pub const CONTROL_SM_H: f32 = 22.0;
    pub const KBD_H: f32 = 17.0;
    pub const STATUS_DOT: f32 = 7.0;
    pub const COMPARE_CHIP_H: f32 = 26.0;
    pub const COMPARE_BAR_H: f32 = 32.0;
    pub const COMPARE_ARROW_MARGIN: f32 = 6.0;
    pub const SETTINGS_SHEET_W: f32 = 980.0;
    pub const SETTINGS_SHEET_H: f32 = 600.0;
    pub const SETTINGS_SHEET_NAV_W: f32 = 210.0;
    pub const SETTINGS_PREVIEW_W: f32 = 330.0;
    pub const SETTINGS_SLIDER_W: f32 = 220.0;
    pub const HISTORY_ROW_H: f32 = 46.0;
    pub const AVATAR_SM: f32 = 20.0;
    pub const AVATAR_MD: f32 = 28.0;
    pub const STATUS_BADGE: f32 = 15.0;
    pub const PROGRESS_H: f32 = 4.0;
}

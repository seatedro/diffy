pub mod commands;
pub mod delivery;
pub mod diff_slots;
pub mod mutations;
pub mod navigation;
pub mod review;
pub mod route;
pub mod shell;
#[cfg(test)]
pub mod testing;
#[cfg(test)]
mod tests;
pub mod text_targets;
pub mod theme;
pub mod tokens;
pub mod viewport;

use quark_app::{RunError, TrafficLights, UiAdapter, WindowChrome, WindowOptions};
use quark_ui::theme::{Theme, ThemeMode};

pub use shell::{DiffyApp, EffectDispatcher};
use theme::ThemePair;

#[derive(Debug, Clone, PartialEq)]
pub struct RunOptions {
    pub title: String,
    pub size: (f64, f64),
    pub min_size: (f64, f64),
    pub theme_name: String,
    pub theme_mode: Option<ThemeMode>,
    pub ui_scale: f32,
}

impl Default for RunOptions {
    fn default() -> Self {
        Self {
            title: crate::platform::startup::app_display_name(),
            size: tokens::Shell::DEFAULT_WINDOW,
            min_size: tokens::Shell::MIN_WINDOW,
            theme_name: theme::DEFAULT_THEME.to_owned(),
            theme_mode: None,
            ui_scale: 1.0,
        }
    }
}

pub fn run<D: EffectDispatcher>(options: RunOptions, dispatcher: D) -> Result<(), RunError> {
    let registry = crate::core::themes::ThemeRegistry::load();
    let themes = ThemePair::load(&registry, &options.theme_name, options.ui_scale);
    let window = window_options(&options, &themes.dark);
    let app = DiffyApp::new(dispatcher, options.title.clone());
    quark_app::run(
        adapter(app, &options.title, themes, options.theme_mode),
        window,
    )
}

pub fn adapter<D: EffectDispatcher>(
    app: DiffyApp<D>,
    title: &str,
    themes: ThemePair,
    mode: Option<ThemeMode>,
) -> UiAdapter<DiffyApp<D>> {
    let adapter = UiAdapter::new(app, title);
    match mode {
        None => adapter.with_themes(themes.light, themes.dark),
        Some(mode) => adapter.with_theme(themes.get(mode).clone()),
    }
}

fn window_options(options: &RunOptions, theme: &Theme) -> WindowOptions {
    let macos = cfg!(target_os = "macos");
    WindowOptions {
        title: options.title.clone(),
        size: options.size,
        min_size: Some(options.min_size),
        chrome: if macos {
            WindowChrome::Custom
        } else {
            WindowChrome::System
        },
        traffic_lights: macos.then_some(TrafficLights {
            left_margin: tokens::Shell::TRAFFIC_LIGHT_MARGIN,
            center_y: theme.metrics.title_bar_height / 2.0,
        }),
        ..WindowOptions::default()
    }
}

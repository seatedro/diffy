pub mod commands;
pub mod delivery;
pub mod diff_slots;
pub mod icons;
pub mod mutations;
pub mod navigation;
pub mod review;
pub mod route;
pub mod runtime;
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

pub fn start() -> Result<(), Box<dyn std::error::Error>> {
    let options = crate::platform::startup::StartupOptions::load();
    crate::platform::logging::init_logging(options.log_debug);
    let settings_store = crate::platform::persistence::SettingsStore::new_default();
    let settings = settings_store.load()?;
    let dispatcher = runtime::RuntimeDispatcher::new(
        crate::apprt::services::AppServices::new(settings_store),
        crate::apprt::runtime::RuntimeOptions {
            keyring_enabled: options.keyring_enabled,
            github_token_store: options.github_token_store,
        },
    );
    let run_options = RunOptions {
        theme_name: settings.theme_name.clone(),
        theme_mode: Some(match settings.theme_mode {
            crate::core::theme_types::ThemeMode::Dark => ThemeMode::Dark,
            crate::core::theme_types::ThemeMode::Light => ThemeMode::Light,
        }),
        ui_scale: f32::from(settings.ui_scale_pct) / 100.0,
        ..RunOptions::default()
    };
    let startup = crate::contracts::shell::AppStartup { options, settings };
    run_with(
        run_options,
        DiffyApp::new(dispatcher, crate::platform::startup::app_display_name())
            .with_startup(startup),
    )?;
    Ok(())
}

pub fn run<D: EffectDispatcher>(options: RunOptions, dispatcher: D) -> Result<(), RunError> {
    let app = DiffyApp::new(dispatcher, options.title.clone());
    run_with(options, app)
}

fn run_with<D: EffectDispatcher>(options: RunOptions, app: DiffyApp<D>) -> Result<(), RunError> {
    let registry = crate::core::themes::ThemeRegistry::load();
    let themes = ThemePair::load(&registry, &options.theme_name, options.ui_scale);
    let window = window_options(&options, &themes.dark);
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

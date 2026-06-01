pub mod accessibility;
pub mod animation;
pub mod app;
pub mod components;
pub mod design;
pub mod element;
pub mod hud;
// Dev/test-only verification substrate (fixtures + no-GPU render path). Gated so
// its fixture data never compiles into a shipping binary; the headless example
// enables `headless-render`, tests get it via `cfg(test)`.
#[cfg(any(test, feature = "headless-render"))]
pub mod harness;
pub mod icons;
pub mod overlays;
pub mod palette;
pub mod settings_page;
pub mod shell;
pub mod sidebar;
pub mod state;
pub mod status_bar;
pub mod style;
pub mod symbols;
pub mod theme;
pub mod title_bar;
pub mod toolbar;
pub mod vcs;
pub mod virtual_list;
pub mod window_chrome;

pub use app::run;

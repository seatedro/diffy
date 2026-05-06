#![recursion_limit = "4096"]

#[global_allocator]
static GLOBAL_ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

pub mod actions;
pub mod ai;
pub mod apprt;
pub mod core;
pub mod editor;
pub mod effects;
pub mod events;
pub mod fonts;
pub mod hot_reload;
pub mod input;
pub mod platform;
pub mod render;
pub mod ui;

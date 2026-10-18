#![recursion_limit = "4096"]

#[global_allocator]
static GLOBAL_ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

pub mod ai;
pub mod app;
pub mod apprt;
pub mod contracts;
pub mod core;
pub mod fonts;
pub mod platform;

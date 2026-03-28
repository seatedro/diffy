#![recursion_limit = "4096"]

#[cfg(feature = "qt")]
#[macro_use]
extern crate cpp;

#[cfg(feature = "qt")]
pub mod app;
pub mod core;

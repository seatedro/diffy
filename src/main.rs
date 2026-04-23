#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

use diffy::ui;

fn main() {
    if let Err(error) = ui::run() {
        eprintln!("native startup failed: {error}");
        std::process::exit(1);
    }
}

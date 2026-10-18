#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

fn main() {
    if let Err(error) = diffy::app::start() {
        eprintln!("native startup failed: {error}");
        std::process::exit(1);
    }
}

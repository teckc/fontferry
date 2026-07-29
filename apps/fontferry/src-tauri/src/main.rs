#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    if let Err(error) = fontferry_app::run() {
        eprintln!("FontFerry: {error:#}");
        std::process::exit(1);
    }
}

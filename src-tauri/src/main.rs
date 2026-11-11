// Prevents an extra console window on Windows in release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // Crash-isolated plugin scanning: the app re-runs itself as a scanner child.
    if let Some(code) = gs_engine::plugins::scan_child_from_args() {
        std::process::exit(code);
    }
    guitar_studio_lib::run()
}

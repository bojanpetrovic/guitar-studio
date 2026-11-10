// Prevents an extra console window on Windows in release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // Crash-isolated plugin scanning: the app re-runs itself as a scanner child.
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("--scan-plugins") {
        let format = args.get(2).map(String::as_str).unwrap_or("");
        std::process::exit(gs_engine::plugins::run_scan_child(format));
    }
    guitar_studio_lib::run()
}

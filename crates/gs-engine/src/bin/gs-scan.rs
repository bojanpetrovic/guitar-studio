//! The plugin scanner the Guitar Studio plugin ships with: run as
//! `gs-scan --scan-plugins <format>`, it scans one format and prints the result (see
//! `gs_engine::plugins::scan_child_from_args`). A separate process, so a plugin that
//! crashes while being scanned can't take the DAW down with it.

fn main() {
    match gs_engine::plugins::scan_child_from_args() {
        Some(code) => std::process::exit(code),
        None => {
            eprintln!("usage: gs-scan --scan-plugins <vst3|au|au3|clap>");
            std::process::exit(2);
        }
    }
}

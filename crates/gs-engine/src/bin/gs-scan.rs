//! The plugin scanner the Guitar Studio plugin ships with: run as
//! `gs-scan --scan-plugins <format>`, it scans one format and prints the result (see
//! `gs_engine::plugins::run_scan_child`). A separate process, so a plugin that crashes
//! while being scanned can't take the DAW down with it.

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) != Some("--scan-plugins") {
        eprintln!("usage: gs-scan --scan-plugins <vst3|au|au3|clap>");
        std::process::exit(2);
    }
    let format = args.get(2).map(String::as_str).unwrap_or("");
    std::process::exit(gs_engine::plugins::run_scan_child(format));
}

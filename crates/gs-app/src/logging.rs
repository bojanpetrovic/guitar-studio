//! A minimal logger: `log` records to stderr (the terminal under `make dev`, Console.app
//! for a bundled app), or to a file (the plugin: a DAW's stderr goes nowhere).
//! `RUST_LOG=debug` (or warn, error, …) changes the level.

use log::{Level, LevelFilter, Log, Metadata, Record};
use parking_lot::Mutex;
use std::io::Write;
use std::path::Path;

struct Logger {
    file: Option<Mutex<std::fs::File>>,
}

/// Our crates; everyone else's records only show at warn and above.
const OURS: &[&str] = &["guitar_studio", "gs_engine", "gs_app", "gs_plugin"];

impl Log for Logger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level() <= log::max_level()
    }

    fn log(&self, record: &Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        // Library chatter (wry, tao, axum…) only at warn and above.
        if record.level() > Level::Warn && !OURS.iter().any(|t| record.target().starts_with(t)) {
            return;
        }
        let line = format!("[{} {}] {}", record.level(), record.target(), record.args());
        match &self.file {
            Some(file) => {
                let _ = writeln!(file.lock(), "{line}");
            }
            None => eprintln!("{line}"),
        }
    }

    fn flush(&self) {}
}

/// Log to stderr, or append to `file` when given. The first call wins (a plugin host
/// may load several instances into one process).
pub fn init(file: Option<&Path>) {
    let file = file.and_then(|path| {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .ok()
            .map(Mutex::new)
    });
    let level = std::env::var("RUST_LOG")
        .ok()
        .and_then(|v| v.parse::<LevelFilter>().ok())
        .unwrap_or(LevelFilter::Info);
    if log::set_boxed_logger(Box::new(Logger { file })).is_ok() {
        log::set_max_level(level);
    }
}

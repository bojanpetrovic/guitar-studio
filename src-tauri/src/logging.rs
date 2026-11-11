//! A minimal logger: `log` records to stderr (the terminal under `make dev`, Console.app
//! for a bundled app). `RUST_LOG=debug` (or warn, error, …) changes the level.

use log::{Level, LevelFilter, Log, Metadata, Record};

struct Stderr;

impl Log for Stderr {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level() <= log::max_level()
    }

    fn log(&self, record: &Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        // Library chatter (wry, tao, axum…) only at warn and above.
        if record.level() > Level::Warn
            && !record.target().starts_with("guitar_studio")
            && !record.target().starts_with("gs_engine")
        {
            return;
        }
        eprintln!("[{} {}] {}", record.level(), record.target(), record.args());
    }

    fn flush(&self) {}
}

pub fn init() {
    let level = std::env::var("RUST_LOG")
        .ok()
        .and_then(|v| v.parse::<LevelFilter>().ok())
        .unwrap_or(LevelFilter::Info);
    if log::set_logger(&Stderr).is_ok() {
        log::set_max_level(level);
    }
}

//! Guitar Studio audio engine.
//!
//! Threading model:
//! - **Control thread** (UI / Tauri commands) owns [`studio::Studio`]: the editable
//!   graph model, plugin instances, and settings. Every graph edit compiles a fresh,
//!   fully-allocated [`graph::Schedule`] and sends it to the audio thread.
//! - **Audio thread** (CoreAudio output callback) swaps in new schedules, runs them,
//!   and hands old ones back through a ring buffer so they are freed off the RT thread.
//! - **Tuner thread** analyses a tap of the guitar input (McLeod pitch method).

pub mod audio;
pub mod graph;
pub mod metronome;
pub mod nodes;
pub mod plugins;
pub mod studio;
pub mod tuner;
pub mod util;

pub use studio::Studio;

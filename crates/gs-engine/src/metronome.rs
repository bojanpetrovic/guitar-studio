//! Sample-accurate metronome, rendered on the audio thread.

use crate::util::AtomicF32;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MetronomeSettings {
    pub enabled: bool,
    pub bpm: f32,
    pub beats_per_bar: u32,
    /// Clicks per beat (1 = quarters, 2 = eighths, 3 = triplets, 4 = sixteenths).
    pub subdivision: u32,
    /// 0..1
    pub volume: f32,
}

impl Default for MetronomeSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            bpm: 120.0,
            beats_per_bar: 4,
            subdivision: 1,
            volume: 0.6,
        }
    }
}

#[derive(Debug, Default)]
pub struct MetronomeShared {
    pub enabled: AtomicBool,
    pub bpm: AtomicF32,
    pub beats_per_bar: AtomicU32,
    pub subdivision: AtomicU32,
    pub volume: AtomicF32,
    /// Beat within the bar that last sounded (for the UI).
    pub beat: AtomicU32,
    /// Increments on every beat (UI uses it to trigger a flash).
    pub beat_counter: AtomicU64,
}

impl MetronomeShared {
    pub fn apply(&self, s: &MetronomeSettings) {
        self.bpm.store(s.bpm.clamp(20.0, 400.0));
        self.beats_per_bar
            .store(s.beats_per_bar.clamp(1, 16), Ordering::Relaxed);
        self.subdivision
            .store(s.subdivision.clamp(1, 8), Ordering::Relaxed);
        self.volume.store(s.volume.clamp(0.0, 1.0));
        self.enabled.store(s.enabled, Ordering::Relaxed);
    }

    pub fn settings(&self) -> MetronomeSettings {
        MetronomeSettings {
            enabled: self.enabled.load(Ordering::Relaxed),
            bpm: self.bpm.load(),
            beats_per_bar: self.beats_per_bar.load(Ordering::Relaxed),
            subdivision: self.subdivision.load(Ordering::Relaxed),
            volume: self.volume.load(),
        }
    }
}

/// Audio-thread state.
pub struct Metronome {
    was_enabled: bool,
    samples_to_tick: f64,
    beat: u32,
    sub: u32,
    // click voice
    env: f32,
    amp: f32,
    phase: f32,
    freq: f32,
}

impl Default for Metronome {
    fn default() -> Self {
        Self {
            was_enabled: false,
            samples_to_tick: 0.0,
            beat: 0,
            sub: 0,
            env: 0.0,
            amp: 0.0,
            phase: 0.0,
            freq: 1000.0,
        }
    }
}

impl Metronome {
    /// Adds clicks into `out` (mono, mixed into both channels by the caller).
    pub fn process(&mut self, shared: &MetronomeShared, sample_rate: f32, out: &mut [f32]) {
        let enabled = shared.enabled.load(Ordering::Relaxed);
        if enabled && !self.was_enabled {
            // Start on a downbeat, immediately.
            self.samples_to_tick = 0.0;
            self.beat = 0;
            self.sub = 0;
        }
        self.was_enabled = enabled;
        if !enabled && self.env < 1e-4 {
            return;
        }

        let bpm = shared.bpm.load().max(1.0) as f64;
        let subdivision = shared.subdivision.load(Ordering::Relaxed).max(1);
        let beats_per_bar = shared.beats_per_bar.load(Ordering::Relaxed).max(1);
        let volume = shared.volume.load();
        let samples_per_tick = sample_rate as f64 * 60.0 / bpm / subdivision as f64;
        let decay = (-1.0 / (0.025 * sample_rate)).exp(); // ~25 ms click
        let tau = std::f32::consts::TAU;

        for s in out.iter_mut() {
            if enabled && self.samples_to_tick <= 0.0 {
                let (freq, amp) = if self.sub == 0 && self.beat == 0 {
                    (1760.0, 1.0)
                } else if self.sub == 0 {
                    (1320.0, 0.7)
                } else {
                    (990.0, 0.35)
                };
                self.freq = freq;
                self.amp = amp;
                self.env = 1.0;
                self.phase = 0.0;
                if self.sub == 0 {
                    shared.beat.store(self.beat, Ordering::Relaxed);
                    shared.beat_counter.fetch_add(1, Ordering::Relaxed);
                }
                self.sub += 1;
                if self.sub >= subdivision {
                    self.sub = 0;
                    self.beat = (self.beat + 1) % beats_per_bar;
                }
                self.samples_to_tick += samples_per_tick;
            }
            self.samples_to_tick -= 1.0;

            if self.env > 1e-4 {
                *s += self.phase.sin() * self.env * self.amp * volume * 0.5;
                self.phase = (self.phase + tau * self.freq / sample_rate) % tau;
                self.env *= decay;
            }
        }
    }
}

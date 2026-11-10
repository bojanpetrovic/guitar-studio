//! Chromatic tuner: McLeod Pitch Method (normalised square difference function)
//! running on its own analysis thread, fed from a lock-free tap of the guitar input.

use crate::util::AtomicF32;
use parking_lot::Mutex;
use serde::Serialize;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

const NOTE_NAMES: [&str; 12] = [
    "C", "C♯", "D", "D♯", "E", "F", "F♯", "G", "G♯", "A", "A♯", "B",
];
const MIN_FREQ: f32 = 50.0; // below drop-A / 7-string low B
const MAX_FREQ: f32 = 1400.0;
const WINDOW: usize = 2048; // after decimation to ~24 kHz ≈ 85 ms
const HOP: usize = 512;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TunerReading {
    pub frequency: f32,
    pub note: &'static str,
    pub octave: i32,
    /// -50..+50
    pub cents: f32,
    /// 0..1 confidence from the NSDF peak height.
    pub clarity: f32,
}

#[derive(Debug)]
pub struct TunerShared {
    pub enabled: AtomicBool,
    pub reference_a4: AtomicF32,
    pub reading: Mutex<Option<TunerReading>>,
}

impl Default for TunerShared {
    fn default() -> Self {
        Self {
            enabled: AtomicBool::new(true),
            reference_a4: AtomicF32::new(440.0),
            reading: Mutex::new(None),
        }
    }
}

/// Owns the analysis thread; stops it on drop.
pub struct TunerThread {
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl Drop for TunerThread {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

pub fn spawn(
    shared: Arc<TunerShared>,
    mut rx: rtrb::Consumer<f32>,
    sample_rate: f32,
) -> TunerThread {
    let stop = Arc::new(AtomicBool::new(false));
    let stop2 = stop.clone();
    let handle = std::thread::Builder::new()
        .name("gs-tuner".into())
        .spawn(move || {
            let decim = ((sample_rate / 24_000.0).round() as usize).max(1);
            let fs = sample_rate / decim as f32;
            let mut ring = vec![0.0f32; WINDOW];
            let mut ring_pos = 0usize;
            let mut window = vec![0.0f32; WINDOW];
            let mut fresh = 0usize;
            let mut acc = 0.0f32;
            let mut acc_n = 0usize;
            let mut history: Vec<f32> = Vec::with_capacity(5);
            let mut last_signal = Instant::now();
            let mut hp = HighPass::new(40.0, fs);

            while !stop2.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(8));
                let available = rx.slots();
                if available == 0 {
                    continue;
                }
                let Ok(chunk) = rx.read_chunk(available) else {
                    continue;
                };
                let (a, b) = chunk.as_slices();
                for &s in a.iter().chain(b) {
                    // Box-filter decimation (cheap anti-alias, fine for pitch work).
                    acc += s;
                    acc_n += 1;
                    if acc_n == decim {
                        ring[ring_pos] = hp.process(acc / decim as f32);
                        ring_pos = (ring_pos + 1) % WINDOW;
                        acc = 0.0;
                        acc_n = 0;
                        fresh += 1;
                    }
                }
                chunk.commit_all();

                if fresh < HOP {
                    continue;
                }
                fresh = 0;
                let (newer, older) = ring.split_at(ring_pos);
                window[..older.len()].copy_from_slice(older);
                window[older.len()..].copy_from_slice(newer);
                if !shared.enabled.load(Ordering::Relaxed) {
                    history.clear();
                    *shared.reading.lock() = None;
                    continue;
                }

                let rms = (window.iter().map(|x| x * x).sum::<f32>() / WINDOW as f32).sqrt();
                let detected = if rms > 0.003 { mpm(&window, fs) } else { None };
                match detected {
                    Some((freq, clarity)) if clarity > 0.8 => {
                        // New note? Reset smoothing when we jump by more than a semitone.
                        if let Some(&last) = history.last()
                            && (12.0 * (freq / last).log2()).abs() > 1.0
                        {
                            history.clear();
                        }
                        if history.len() == 5 {
                            history.remove(0);
                        }
                        history.push(freq);
                        let mut sorted = history.clone();
                        sorted.sort_by(f32::total_cmp);
                        let median = sorted[sorted.len() / 2];
                        *shared.reading.lock() =
                            Some(reading(median, clarity, shared.reference_a4.load()));
                        last_signal = Instant::now();
                    }
                    _ => {
                        if last_signal.elapsed() > Duration::from_millis(400) {
                            history.clear();
                            *shared.reading.lock() = None;
                        }
                    }
                }
            }
        })
        .expect("spawn tuner thread");
    TunerThread {
        stop,
        handle: Some(handle),
    }
}

fn reading(freq: f32, clarity: f32, a4: f32) -> TunerReading {
    let midi = 69.0 + 12.0 * (freq / a4).log2();
    let nearest = midi.round();
    let idx = (nearest as i32).rem_euclid(12) as usize;
    TunerReading {
        frequency: freq,
        note: NOTE_NAMES[idx],
        octave: (nearest as i32).div_euclid(12) - 1,
        cents: (midi - nearest) * 100.0,
        clarity,
    }
}

/// McLeod Pitch Method. Returns `(frequency_hz, clarity)`.
pub fn mpm(x: &[f32], fs: f32) -> Option<(f32, f32)> {
    let n = x.len();
    let tau_min = (fs / MAX_FREQ).floor() as usize;
    let tau_max = ((fs / MIN_FREQ).ceil() as usize).min(n / 2);
    if tau_max <= tau_min + 2 {
        return None;
    }

    // NSDF: n'(τ) = 2·r(τ) / m(τ), with m computed incrementally.
    let mut nsdf = vec![0.0f32; tau_max + 1];
    let mut m: f32 = x.iter().map(|v| v * v).sum::<f32>() * 2.0;
    for tau in 0..=tau_max {
        if tau > 0 {
            m -= x[n - tau] * x[n - tau] + x[tau - 1] * x[tau - 1];
        }
        let r: f32 = x[..n - tau].iter().zip(&x[tau..]).map(|(a, b)| a * b).sum();
        nsdf[tau] = if m > 1e-9 { 2.0 * r / m } else { 0.0 };
    }

    // Key maxima: highest point between each positive-going and negative-going zero crossing.
    let mut peaks: Vec<(usize, f32)> = Vec::new();
    let mut tau = 1;
    while tau < tau_max && nsdf[tau] > 0.0 {
        tau += 1; // skip the initial lobe around τ = 0
    }
    while tau < tau_max {
        while tau < tau_max && nsdf[tau] <= 0.0 {
            tau += 1;
        }
        let mut best: Option<(usize, f32)> = None;
        while tau < tau_max && nsdf[tau] > 0.0 {
            if nsdf[tau] > nsdf[tau - 1]
                && nsdf[tau] >= nsdf[tau + 1]
                && best.is_none_or(|(_, v)| nsdf[tau] > v)
            {
                best = Some((tau, nsdf[tau]));
            }
            tau += 1;
        }
        if let Some(p) = best {
            peaks.push(p);
        }
    }

    let highest = peaks.iter().map(|p| p.1).fold(0.0f32, f32::max);
    if highest <= 0.0 {
        return None;
    }
    let threshold = 0.9 * highest;
    let &(t, v) = peaks.iter().find(|p| p.1 >= threshold && p.0 >= tau_min)?;

    // Parabolic interpolation around the chosen peak.
    let (a, b, c) = (nsdf[t - 1], v, nsdf[t + 1]);
    let denom = a - 2.0 * b + c;
    let shift = if denom.abs() > 1e-12 {
        0.5 * (a - c) / denom
    } else {
        0.0
    };
    let period = t as f32 + shift;
    let clarity = b - 0.25 * (a - c) * shift;
    Some((fs / period, clarity.min(1.0)))
}

/// One-pole high-pass to strip DC and rumble.
struct HighPass {
    a: f32,
    prev_x: f32,
    prev_y: f32,
}

impl HighPass {
    fn new(cutoff: f32, fs: f32) -> Self {
        let rc = 1.0 / (std::f32::consts::TAU * cutoff);
        Self {
            a: rc / (rc + 1.0 / fs),
            prev_x: 0.0,
            prev_y: 0.0,
        }
    }

    fn process(&mut self, x: f32) -> f32 {
        let y = self.a * (self.prev_y + x - self.prev_x);
        self.prev_x = x;
        self.prev_y = y;
        y
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn guitar_like(freq: f32, fs: f32) -> Vec<f32> {
        (0..WINDOW)
            .map(|i| {
                let t = i as f32 / fs;
                let w = std::f32::consts::TAU * freq * t;
                0.6 * w.sin()
                    + 0.3 * (2.0 * w).sin()
                    + 0.15 * (3.0 * w).sin()
                    + 0.1 * (5.0 * w).sin()
            })
            .collect()
    }

    #[test]
    fn detects_guitar_strings() {
        let fs = 24_000.0;
        // Low B (7-string), E2, A2, D3, G3, B3, E4
        for f in [61.74, 82.41, 110.0, 146.83, 196.0, 246.94, 329.63] {
            let (got, clarity) = mpm(&guitar_like(f, fs), fs).expect("pitch");
            let cents = 1200.0 * (got / f).log2();
            assert!(cents.abs() < 2.0, "{f} Hz → {got} Hz ({cents:.2} cents)");
            assert!(clarity > 0.9);
        }
    }

    #[test]
    fn names_notes() {
        let r = reading(82.41, 1.0, 440.0);
        assert_eq!((r.note, r.octave), ("E", 2));
        assert!(r.cents.abs() < 1.0);
        let r = reading(445.0, 1.0, 440.0);
        assert_eq!(r.note, "A");
        assert!((r.cents - 19.56).abs() < 0.1);
    }
}
